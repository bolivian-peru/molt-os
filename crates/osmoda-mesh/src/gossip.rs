use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::messages::MeshMessage;
use crate::MeshState;

/// Gossip protocol messages for room synchronization.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "gossip_type", rename_all = "snake_case")]
pub enum GossipMessage {
    /// Request messages from a room since a given timestamp.
    RoomSync {
        room_id: String,
        messages_since: Option<String>,
    },
    /// Reply with messages for sync.
    RoomSyncReply {
        room_id: String,
        messages: Vec<SyncMessage>,
    },
    /// Notify that a peer joined a room.
    RoomJoinNotify {
        room_id: String,
        peer_id: String,
    },
    /// Notify that a peer left a room.
    RoomLeaveNotify {
        room_id: String,
        peer_id: String,
    },
}

/// A message payload used in gossip sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMessage {
    pub sender: String,
    pub content: String,
    pub timestamp: String,
    pub msg_hash: String,
}

/// Forward a room message to all connected room members except the sender.
pub async fn forward_room_message(
    state: &Arc<Mutex<MeshState>>,
    room_id: &str,
    sender_peer_id: &str,
    from: &str,
    text: &str,
) {
    let st = state.lock().await;

    // Get room members from the room store
    let members = match &st.room_store {
        Some(store) => match store.get_members(room_id) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, room_id = %room_id, "failed to get room members for forwarding");
                return;
            }
        },
        None => return,
    };

    // Forward to each connected member except the sender
    for member in &members {
        if member.peer_id == sender_peer_id {
            continue;
        }

        if let Some(conn) = st.connections.get(&member.peer_id) {
            let msg = MeshMessage::Chat {
                from: from.to_string(),
                text: text.to_string(),
                room_id: Some(room_id.to_string()),
            };
            let conn = conn.clone();
            let peer_id = member.peer_id.clone();
            tokio::spawn(async move {
                if let Err(e) = conn.send_message(&msg).await {
                    tracing::debug!(
                        peer_id = %peer_id,
                        error = %e,
                        "failed to forward room message (non-fatal)"
                    );
                }
            });
        }
    }
}

/// Request sync of missed messages from a peer.
pub async fn request_sync(
    state: &Arc<Mutex<MeshState>>,
    peer_id: &str,
    room_id: &str,
) {
    let st = state.lock().await;

    let since = match &st.room_store {
        Some(store) => store.latest_timestamp(room_id).unwrap_or(None),
        None => None,
    };

    if let Some(conn) = st.connections.get(peer_id) {
        let gossip = GossipMessage::RoomSync {
            room_id: room_id.to_string(),
            messages_since: since,
        };
        let payload = match serde_json::to_string(&gossip) {
            Ok(p) => p,
            Err(_) => return,
        };
        let msg = MeshMessage::Chat {
            from: "__gossip__".to_string(),
            text: payload,
            room_id: Some("__gossip_channel__".to_string()),
        };

        let conn = conn.clone();
        tokio::spawn(async move {
            if let Err(e) = conn.send_message(&msg).await {
                tracing::debug!(error = %e, "failed to send gossip sync request");
            }
        });
    }
}

/// Handle an incoming gossip message.
pub async fn handle_gossip(
    state: &Arc<Mutex<MeshState>>,
    peer_id: &str,
    gossip_text: &str,
) -> bool {
    let gossip: GossipMessage = match serde_json::from_str(gossip_text) {
        Ok(g) => g,
        Err(_) => return false,
    };

    match gossip {
        GossipMessage::RoomSync {
            room_id,
            messages_since,
        } => {
            handle_sync_request(state, peer_id, &room_id, messages_since.as_deref()).await;
            true
        }
        GossipMessage::RoomSyncReply { room_id, messages } => {
            handle_sync_reply(state, &room_id, messages).await;
            true
        }
        GossipMessage::RoomJoinNotify { room_id, peer_id: joining_peer } => {
            let st = state.lock().await;
            if let Some(store) = &st.room_store {
                let _ = store.join_room(&room_id, &joining_peer);
            }
            true
        }
        GossipMessage::RoomLeaveNotify { room_id, peer_id: leaving_peer } => {
            let st = state.lock().await;
            if let Some(store) = &st.room_store {
                let _ = store.leave_room(&room_id, &leaving_peer);
            }
            true
        }
    }
}

/// Respond to a sync request with missed messages.
async fn handle_sync_request(
    state: &Arc<Mutex<MeshState>>,
    peer_id: &str,
    room_id: &str,
    since: Option<&str>,
) {
    let st = state.lock().await;

    let messages = match &st.room_store {
        Some(store) => match store.get_history(room_id, since, 100) {
            Ok(msgs) => msgs,
            Err(e) => {
                tracing::warn!(error = %e, "failed to get room history for sync");
                return;
            }
        },
        None => return,
    };

    let sync_msgs: Vec<SyncMessage> = messages
        .into_iter()
        .map(|m| SyncMessage {
            sender: m.sender,
            content: m.content,
            timestamp: m.timestamp,
            msg_hash: m.msg_hash,
        })
        .collect();

    if sync_msgs.is_empty() {
        return;
    }

    let reply = GossipMessage::RoomSyncReply {
        room_id: room_id.to_string(),
        messages: sync_msgs,
    };

    let payload = match serde_json::to_string(&reply) {
        Ok(p) => p,
        Err(_) => return,
    };

    if let Some(conn) = st.connections.get(peer_id) {
        let msg = MeshMessage::Chat {
            from: "__gossip__".to_string(),
            text: payload,
            room_id: Some("__gossip_channel__".to_string()),
        };
        let conn = conn.clone();
        tokio::spawn(async move {
            if let Err(e) = conn.send_message(&msg).await {
                tracing::debug!(error = %e, "failed to send gossip sync reply");
            }
        });
    }
}

/// Apply synced messages from a peer.
async fn handle_sync_reply(
    state: &Arc<Mutex<MeshState>>,
    room_id: &str,
    messages: Vec<SyncMessage>,
) {
    let st = state.lock().await;
    let store = match &st.room_store {
        Some(s) => s,
        None => return,
    };

    let mut new_count = 0;
    for msg in messages {
        match store.store_message(room_id, &msg.sender, &msg.content, &msg.timestamp) {
            Ok(true) => new_count += 1,
            Ok(false) => {} // duplicate, skip
            Err(e) => {
                tracing::warn!(error = %e, "failed to store synced message");
            }
        }
    }

    if new_count > 0 {
        tracing::info!(room_id = %room_id, new_messages = new_count, "gossip sync applied");
    }
}

/// Background task that syncs rooms with connected peers on reconnect.
pub async fn sync_rooms_with_peer(state: Arc<Mutex<MeshState>>, peer_id: String) {
    let room_ids: Vec<String> = {
        let st = state.lock().await;
        match &st.room_store {
            Some(store) => match store.list_rooms() {
                Ok(rooms) => rooms.into_iter().map(|r| r.id).collect(),
                Err(_) => return,
            },
            None => return,
        }
    };

    for room_id in room_ids {
        request_sync(&state, &peer_id, &room_id).await;
        // Small delay between sync requests to avoid flooding
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gossip_message_serde_roundtrip() {
        let msg = GossipMessage::RoomSync {
            room_id: "room-1".to_string(),
            messages_since: Some("2026-01-01T00:00:00Z".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: GossipMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            GossipMessage::RoomSync {
                room_id,
                messages_since,
            } => {
                assert_eq!(room_id, "room-1");
                assert_eq!(
                    messages_since,
                    Some("2026-01-01T00:00:00Z".to_string())
                );
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_sync_reply_serde() {
        let msg = GossipMessage::RoomSyncReply {
            room_id: "room-1".to_string(),
            messages: vec![SyncMessage {
                sender: "peer-a".to_string(),
                content: "hello".to_string(),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                msg_hash: "abc123".to_string(),
            }],
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: GossipMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            GossipMessage::RoomSyncReply { room_id, messages } => {
                assert_eq!(room_id, "room-1");
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].content, "hello");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_join_leave_notify_serde() {
        let join = GossipMessage::RoomJoinNotify {
            room_id: "room-1".to_string(),
            peer_id: "peer-x".to_string(),
        };
        let json = serde_json::to_string(&join).unwrap();
        assert!(json.contains("room_join_notify"));

        let leave = GossipMessage::RoomLeaveNotify {
            room_id: "room-1".to_string(),
            peer_id: "peer-x".to_string(),
        };
        let json = serde_json::to_string(&leave).unwrap();
        assert!(json.contains("room_leave_notify"));
    }
}
