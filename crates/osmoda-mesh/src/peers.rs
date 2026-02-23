use serde::{Deserialize, Serialize};

/// Connection state of a peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected { since: String },
    Failed { reason: String, at: String },
}

/// Stored information about a known mesh peer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub id: String,
    pub label: String,
    pub noise_static_pubkey: String,
    pub mlkem_encap_key: String,
    pub endpoint: String,
    pub added_at: String,
    pub last_seen: Option<String>,
    pub connection_state: ConnectionState,
}

/// Save peers to a JSON file (same pattern as watch/api.rs).
pub fn save_peers(peers: &[PeerInfo], dir: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = std::path::Path::new(dir).join("peers.json");
    let data = serde_json::to_string_pretty(peers)?;
    std::fs::write(path, data)?;
    Ok(())
}

/// Load peers from a JSON file. Returns empty vec if file doesn't exist.
pub fn load_peers(dir: &str) -> Vec<PeerInfo> {
    let path = std::path::Path::new(dir).join("peers.json");
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(peers) = serde_json::from_str::<Vec<PeerInfo>>(&data) {
                return peers;
            }
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_peer(id: &str, label: &str) -> PeerInfo {
        PeerInfo {
            id: id.to_string(),
            label: label.to_string(),
            noise_static_pubkey: "aabb".to_string(),
            mlkem_encap_key: "ccdd".to_string(),
            endpoint: "127.0.0.1:18800".to_string(),
            added_at: "2026-01-01T00:00:00Z".to_string(),
            last_seen: None,
            connection_state: ConnectionState::Disconnected,
        }
    }

    #[test]
    fn test_load_peers_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let peers = load_peers(dir.path().to_str().unwrap());
        assert!(peers.is_empty());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let dir_str = dir.path().to_str().unwrap();

        let peers = vec![make_peer("p1", "Node Alpha"), make_peer("p2", "Node Beta")];

        save_peers(&peers, dir_str).unwrap();

        let loaded = load_peers(dir_str);
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "p1");
        assert_eq!(loaded[0].label, "Node Alpha");
        assert_eq!(loaded[1].id, "p2");
        assert_eq!(loaded[1].label, "Node Beta");
    }

    #[test]
    fn test_connection_state_serde() {
        let connected = ConnectionState::Connected {
            since: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&connected).unwrap();
        let decoded: ConnectionState = serde_json::from_str(&json).unwrap();
        match decoded {
            ConnectionState::Connected { since } => {
                assert_eq!(since, "2026-01-01T00:00:00Z");
            }
            _ => panic!("wrong variant"),
        }

        let failed = ConnectionState::Failed {
            reason: "timeout".to_string(),
            at: "2026-01-01T00:01:00Z".to_string(),
        };
        let json = serde_json::to_string(&failed).unwrap();
        let decoded: ConnectionState = serde_json::from_str(&json).unwrap();
        match decoded {
            ConnectionState::Failed { reason, at } => {
                assert_eq!(reason, "timeout");
                assert_eq!(at, "2026-01-01T00:01:00Z");
            }
            _ => panic!("wrong variant"),
        }
    }
}
