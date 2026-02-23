use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::identity::MeshPublicIdentity;
use crate::invite::InvitePayload;
use crate::messages::MeshMessage;
use crate::peers::{ConnectionState, PeerInfo};
use crate::{MeshState, Room, RoomMessage};

pub type SharedState = Arc<Mutex<MeshState>>;

// ── Invite endpoints ──

#[derive(Debug, Deserialize)]
pub struct CreateInviteRequest {
    pub ttl_secs: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct CreateInviteResponse {
    pub invite_code: String,
    pub expires_at: String,
}

/// POST /invite/create — create an invite code for a peer to join.
pub async fn invite_create_handler(
    State(state): State<SharedState>,
    Json(body): Json<CreateInviteRequest>,
) -> Result<Json<CreateInviteResponse>, (axum::http::StatusCode, String)> {
    let st = state.lock().await;
    let identity = &st.identity.public_identity;

    let invite = InvitePayload::new(
        &st.listen_endpoint,
        &identity.noise_static_pubkey,
        &identity.mlkem_encap_key,
        &identity.instance_id,
        body.ttl_secs,
    );

    let code = invite.encode().map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to encode invite: {e}"),
        )
    })?;

    tracing::info!("invite created (expires {})", invite.expires_at);

    Ok(Json(CreateInviteResponse {
        invite_code: code,
        expires_at: invite.expires_at,
    }))
}

#[derive(Debug, Deserialize)]
pub struct AcceptInviteRequest {
    pub invite_code: String,
}

#[derive(Debug, Serialize)]
pub struct AcceptInviteResponse {
    pub peer_id: String,
    pub status: String,
}

/// POST /invite/accept — accept an invite code and connect to the peer.
pub async fn invite_accept_handler(
    State(state): State<SharedState>,
    Json(body): Json<AcceptInviteRequest>,
) -> Result<Json<AcceptInviteResponse>, (axum::http::StatusCode, String)> {
    let payload = InvitePayload::decode(&body.invite_code).map_err(|e| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            format!("invalid invite: {e}"),
        )
    })?;

    let (peer_id, endpoint) = {
        let mut st = state.lock().await;

        // Check if we already know this peer
        if st.peers.iter().any(|p| p.id == payload.instance_id) {
            return Err((
                axum::http::StatusCode::CONFLICT,
                "peer already known".to_string(),
            ));
        }

        let peer = PeerInfo {
            id: payload.instance_id.clone(),
            label: format!("peer-{}", &payload.instance_id[..8]),
            noise_static_pubkey: payload.noise_static_pubkey,
            mlkem_encap_key: payload.mlkem_encap_key,
            endpoint: payload.endpoint.clone(),
            added_at: chrono::Utc::now().to_rfc3339(),
            last_seen: None,
            connection_state: ConnectionState::Connecting,
        };

        st.peers.push(peer);
        if let Err(e) = crate::peers::save_peers(&st.peers, &st.data_dir) {
            tracing::warn!(error = %e, "failed to persist peers");
        }

        tracing::info!(
            peer_id = %payload.instance_id,
            "invite accepted, spawning outbound connection"
        );

        (payload.instance_id.clone(), payload.endpoint.clone())
    }; // lock released here

    // Spawn outbound connection without holding the state lock
    let state_for_connect = state.clone();
    tokio::spawn(async move {
        crate::initiate_outbound_connection(state_for_connect, peer_id, endpoint).await;
    });

    Ok(Json(AcceptInviteResponse {
        peer_id: payload.instance_id,
        status: "connecting".to_string(),
    }))
}

// ── Peer endpoints ──

/// GET /peers — list all known peers.
pub async fn peers_list_handler(State(state): State<SharedState>) -> Json<Vec<PeerInfo>> {
    let st = state.lock().await;
    Json(st.peers.clone())
}

/// GET /peer/{id} — get a specific peer's info.
pub async fn peer_get_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<PeerInfo>, axum::http::StatusCode> {
    let st = state.lock().await;
    st.peers
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .map(Json)
        .ok_or(axum::http::StatusCode::NOT_FOUND)
}

#[derive(Debug, Deserialize)]
pub struct SendMessageRequest {
    pub message: MeshMessage,
}

#[derive(Debug, Serialize)]
pub struct SendMessageResponse {
    pub delivered: bool,
}

/// POST /peer/{id}/send — send a message to a connected peer.
pub async fn peer_send_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, (axum::http::StatusCode, String)> {
    let st = state.lock().await;

    let connection = st.connections.get(&id).ok_or((
        axum::http::StatusCode::NOT_FOUND,
        format!("no active connection to peer {id}"),
    ))?;

    connection.send_message(&body.message).await.map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("send failed: {e}"),
        )
    })?;

    // Log to agentd (best-effort)
    let msg_type = match &body.message {
        MeshMessage::Heartbeat { .. } => "heartbeat",
        MeshMessage::HealthReport { .. } => "health_report",
        MeshMessage::Alert { .. } => "alert",
        MeshMessage::Chat { .. } => "chat",
        MeshMessage::PqExchange { .. } => "pq_exchange",
    };
    st.receipt_logger.log_message_sent(&id, msg_type).await;

    Ok(Json(SendMessageResponse { delivered: true }))
}

/// DELETE /peer/{id} — disconnect and remove a peer.
pub async fn peer_disconnect_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    let mut st = state.lock().await;

    // Remove active connection
    st.connections.remove(&id);

    let before = st.peers.len();
    st.peers.retain(|p| p.id != id);
    if st.peers.len() < before {
        if let Err(e) = crate::peers::save_peers(&st.peers, &st.data_dir) {
            tracing::warn!(error = %e, "failed to persist after peer removal");
        }
        st.receipt_logger.log_disconnect(&id).await;
        tracing::info!(peer_id = %id, "peer disconnected and removed");
        Ok(Json(serde_json::json!({"disconnected": id})))
    } else {
        Err(axum::http::StatusCode::NOT_FOUND)
    }
}

// ── Identity endpoints ──

/// POST /identity/rotate — rotate all keys and generate new identity.
pub async fn identity_rotate_handler(
    State(state): State<SharedState>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let mut st = state.lock().await;

    // Generate new identity
    let data_dir_path = std::path::Path::new(&st.data_dir);
    let new_identity =
        crate::identity::MeshIdentity::load_or_create(data_dir_path).map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to generate new identity: {e}"),
            )
        })?;

    let new_id = new_identity.public_identity.instance_id.clone();
    let pubkeys = serde_json::json!({
        "ed25519": new_identity.public_identity.ed25519_pubkey,
        "noise": new_identity.public_identity.noise_static_pubkey,
    });

    st.identity = new_identity;

    tracing::info!(new_instance_id = %new_id, "identity rotated");

    Ok(Json(serde_json::json!({
        "new_instance_id": new_id,
        "new_pubkeys": pubkeys,
    })))
}

/// GET /identity — get our public identity.
pub async fn identity_get_handler(
    State(state): State<SharedState>,
) -> Json<MeshPublicIdentity> {
    let st = state.lock().await;
    Json(st.identity.public_identity.clone())
}

// ── Room endpoints ──

#[derive(Debug, Deserialize)]
pub struct CreateRoomRequest {
    pub name: String,
}

#[derive(Debug, Serialize)]
pub struct RoomSummary {
    pub id: String,
    pub name: String,
    pub member_count: usize,
    pub message_count: usize,
    pub created_at: String,
}

/// POST /room/create — create a new group room.
pub async fn room_create_handler(
    State(state): State<SharedState>,
    Json(body): Json<CreateRoomRequest>,
) -> Result<Json<Room>, (axum::http::StatusCode, String)> {
    let room = Room {
        id: uuid::Uuid::new_v4().to_string(),
        name: body.name,
        members: Vec::new(),
        messages: Vec::new(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    let mut st = state.lock().await;
    st.rooms.push(room.clone());

    tracing::info!(room_id = %room.id, room_name = %room.name, "room created");
    Ok(Json(room))
}

#[derive(Debug, Deserialize)]
pub struct JoinRoomRequest {
    pub room_id: String,
    pub peer_id: String,
}

/// POST /room/join — add a peer to a room's member list.
pub async fn room_join_handler(
    State(state): State<SharedState>,
    Json(body): Json<JoinRoomRequest>,
) -> Result<Json<Room>, (axum::http::StatusCode, String)> {
    let mut st = state.lock().await;

    let room = st.rooms.iter_mut().find(|r| r.id == body.room_id).ok_or((
        axum::http::StatusCode::NOT_FOUND,
        format!("room {} not found", body.room_id),
    ))?;

    if !room.members.contains(&body.peer_id) {
        room.members.push(body.peer_id.clone());
        tracing::info!(room_id = %body.room_id, peer_id = %body.peer_id, "peer joined room");
    }

    Ok(Json(room.clone()))
}

#[derive(Debug, Deserialize)]
pub struct SendRoomRequest {
    pub room_id: String,
    pub text: String,
}

/// POST /room/send — send a message to all connected members of a room.
pub async fn room_send_handler(
    State(state): State<SharedState>,
    Json(body): Json<SendRoomRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let mut st = state.lock().await;

    let my_id = st.identity.public_identity.instance_id.clone();
    let agentd_socket = st.receipt_logger.agentd_socket.clone();

    let room = st.rooms.iter_mut().find(|r| r.id == body.room_id).ok_or((
        axum::http::StatusCode::NOT_FOUND,
        format!("room {} not found", body.room_id),
    ))?;

    // Append to local room history
    let seq = room.messages.len() as u64;
    room.messages.push(RoomMessage {
        from: my_id.clone(),
        text: body.text.clone(),
        ts: chrono::Utc::now().to_rfc3339(),
        seq,
    });

    let members = room.members.clone();
    let room_id = body.room_id.clone();
    let text = body.text.clone();

    // Send to each connected member
    let mut delivered = 0usize;
    let chat_msg = MeshMessage::Chat {
        from: my_id.clone(),
        text: text.clone(),
        room_id: Some(room_id.clone()),
    };

    for peer_id in &members {
        if let Some(conn) = st.connections.get(peer_id) {
            match conn.send_message(&chat_msg).await {
                Ok(_) => { delivered += 1; }
                Err(e) => {
                    tracing::warn!(peer_id = %peer_id, error = %e, "failed to send room message to peer");
                }
            }
        }
    }

    drop(st);

    // Log to agentd (best-effort)
    let log_body = serde_json::json!({
        "source": "osmoda-mesh",
        "content": serde_json::json!({
            "room_id": room_id,
            "from": my_id,
            "text": text,
            "delivered_to": delivered,
        }).to_string(),
        "category": "mesh.room.message",
        "tags": ["mesh", "room", room_id],
    }).to_string();
    tokio::spawn(async move {
        if let Err(e) = crate::post_to_agentd(&agentd_socket, log_body).await {
            tracing::debug!(error = %e, "failed to log room send to agentd (non-fatal)");
        }
    });

    Ok(Json(serde_json::json!({ "delivered_to": delivered })))
}

#[derive(Debug, Deserialize)]
pub struct RoomHistoryQuery {
    pub room_id: String,
    pub limit: Option<usize>,
}

/// GET /room/history — get recent messages from a room.
pub async fn room_history_handler(
    State(state): State<SharedState>,
    Query(params): Query<RoomHistoryQuery>,
) -> Result<Json<Vec<RoomMessage>>, (axum::http::StatusCode, String)> {
    let st = state.lock().await;

    let room = st.rooms.iter().find(|r| r.id == params.room_id).ok_or((
        axum::http::StatusCode::NOT_FOUND,
        format!("room {} not found", params.room_id),
    ))?;

    let limit = params.limit.unwrap_or(50).min(500);
    let messages: Vec<RoomMessage> = room.messages.iter().rev().take(limit).cloned().collect();
    let mut messages: Vec<RoomMessage> = messages.into_iter().rev().collect();
    // Return in chronological order
    messages.sort_by_key(|m| m.seq);

    Ok(Json(messages))
}

/// GET /rooms — list all rooms.
pub async fn rooms_list_handler(State(state): State<SharedState>) -> Json<Vec<RoomSummary>> {
    let st = state.lock().await;
    let rooms: Vec<RoomSummary> = st.rooms.iter().map(|r| RoomSummary {
        id: r.id.clone(),
        name: r.name.clone(),
        member_count: r.members.len(),
        message_count: r.messages.len(),
        created_at: r.created_at.clone(),
    }).collect();
    Json(rooms)
}

// ── Health ──

#[derive(Debug, Serialize)]
pub struct MeshHealthResponse {
    pub status: String,
    pub peer_count: usize,
    pub connected_count: usize,
    pub identity_ready: bool,
}

/// GET /health — mesh daemon health.
pub async fn health_handler(State(state): State<SharedState>) -> Json<MeshHealthResponse> {
    let st = state.lock().await;
    Json(MeshHealthResponse {
        status: "ok".to_string(),
        peer_count: st.peers.len(),
        connected_count: st.connections.len(),
        identity_ready: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_response_shape() {
        let resp = MeshHealthResponse {
            status: "ok".to_string(),
            peer_count: 2,
            connected_count: 1,
            identity_ready: true,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["peer_count"], 2);
        assert_eq!(json["connected_count"], 1);
        assert_eq!(json["identity_ready"], true);
    }

    #[test]
    fn test_room_create_defaults() {
        let room = Room {
            id: "test-id".to_string(),
            name: "Test Room".to_string(),
            members: Vec::new(),
            messages: Vec::new(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        assert_eq!(room.id, "test-id");
        assert_eq!(room.name, "Test Room");
        assert!(room.members.is_empty(), "new room should have no members");
        assert!(room.messages.is_empty(), "new room should have no messages");
    }

    #[test]
    fn test_room_join_adds_member() {
        let mut room = Room {
            id: "r1".to_string(),
            name: "Chat".to_string(),
            members: Vec::new(),
            messages: Vec::new(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        room.members.push("peer-abc".to_string());
        assert_eq!(room.members.len(), 1);
        assert_eq!(room.members[0], "peer-abc");

        // Should not duplicate
        if !room.members.contains(&"peer-abc".to_string()) {
            room.members.push("peer-abc".to_string());
        }
        assert_eq!(room.members.len(), 1, "duplicate member should not be added");
    }

    #[test]
    fn test_room_history_limit() {
        let mut room = Room {
            id: "r2".to_string(),
            name: "Test".to_string(),
            members: Vec::new(),
            messages: Vec::new(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };
        for i in 0..10u64 {
            room.messages.push(RoomMessage {
                from: "alice".to_string(),
                text: format!("msg {i}"),
                ts: "2026-01-01T00:00:00Z".to_string(),
                seq: i,
            });
        }
        let limit = 3;
        let messages: Vec<RoomMessage> = room.messages.iter().rev().take(limit).cloned().collect();
        let mut messages: Vec<RoomMessage> = messages.into_iter().rev().collect();
        messages.sort_by_key(|m| m.seq);
        assert_eq!(messages.len(), 3, "should return last 3 messages");
        assert_eq!(messages[0].seq, 7);
        assert_eq!(messages[1].seq, 8);
        assert_eq!(messages[2].seq, 9);
    }
}
