use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::identity::MeshPublicIdentity;
use crate::invite::InvitePayload;
use crate::messages::MeshMessage;
use crate::peers::{ConnectionState, PeerInfo};
use crate::MeshState;

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

    // Queue connection attempt (the background loop will pick it up)
    st.pending_connections.push(payload.endpoint);

    tracing::info!(
        peer_id = %payload.instance_id,
        "invite accepted, connection queued"
    );

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
        MeshMessage::LedgerSync { .. } => "ledger_sync",
        MeshMessage::Command { .. } => "command",
        MeshMessage::CommandResponse { .. } => "command_response",
        MeshMessage::PeerAnnounce { .. } => "peer_announce",
        MeshMessage::KeyRotation { .. } => "key_rotation",
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
}
