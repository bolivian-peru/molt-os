use serde::{Deserialize, Serialize};

/// Severity levels for mesh alerts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

/// All message types that can be sent over a mesh connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MeshMessage {
    Heartbeat {
        timestamp: String,
    },
    HealthReport {
        hostname: String,
        cpu: f64,
        memory: f64,
        uptime: u64,
    },
    Alert {
        severity: AlertSeverity,
        title: String,
        detail: String,
    },
    Chat {
        from: String,
        text: String,
        /// None = direct message; Some(id) = group room message
        #[serde(skip_serializing_if = "Option::is_none")]
        room_id: Option<String>,
    },
    PqExchange {
        /// base64-encoded ML-KEM ciphertext
        mlkem_ciphertext: String,
    },
}

/// Wire frame: length-prefixed messages for TCP transport.
/// Format: [4 bytes big-endian length][payload]
pub fn encode_frame(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u32;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// Decode a length prefix from a 4-byte buffer.
pub fn decode_frame_length(header: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*header)
}

/// Maximum message size (1 MB).
pub const MAX_MESSAGE_SIZE: u32 = 1_048_576;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serde_roundtrip_heartbeat() {
        let msg = MeshMessage::Heartbeat {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: MeshMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            MeshMessage::Heartbeat { timestamp } => {
                assert_eq!(timestamp, "2026-01-01T00:00:00Z");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_serde_roundtrip_health_report() {
        let msg = MeshMessage::HealthReport {
            hostname: "node-1".to_string(),
            cpu: 45.2,
            memory: 72.1,
            uptime: 86400,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: MeshMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            MeshMessage::HealthReport {
                hostname,
                cpu,
                memory,
                uptime,
            } => {
                assert_eq!(hostname, "node-1");
                assert!((cpu - 45.2).abs() < f64::EPSILON);
                assert!((memory - 72.1).abs() < f64::EPSILON);
                assert_eq!(uptime, 86400);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_serde_roundtrip_alert() {
        let msg = MeshMessage::Alert {
            severity: AlertSeverity::Critical,
            title: "Disk full".to_string(),
            detail: "/ is at 99%".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: MeshMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            MeshMessage::Alert {
                severity,
                title,
                detail,
            } => {
                assert_eq!(severity, AlertSeverity::Critical);
                assert_eq!(title, "Disk full");
                assert_eq!(detail, "/ is at 99%");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_serde_roundtrip_chat_dm() {
        let msg = MeshMessage::Chat {
            from: "admin".to_string(),
            text: "hello peer".to_string(),
            room_id: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        // room_id should be absent when None
        assert!(!json.contains("room_id"), "room_id should be absent for DMs");
        let decoded: MeshMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            MeshMessage::Chat { from, text, room_id } => {
                assert_eq!(from, "admin");
                assert_eq!(text, "hello peer");
                assert!(room_id.is_none());
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_chat_with_room_id() {
        let msg = MeshMessage::Chat {
            from: "agent-a".to_string(),
            text: "hello room".to_string(),
            room_id: Some("room-123".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("room_id"), "room_id should be present");
        let decoded: MeshMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            MeshMessage::Chat { from, text, room_id } => {
                assert_eq!(from, "agent-a");
                assert_eq!(text, "hello room");
                assert_eq!(room_id, Some("room-123".to_string()));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_serde_roundtrip_pq_exchange() {
        let msg = MeshMessage::PqExchange {
            mlkem_ciphertext: "dGVzdA==".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: MeshMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            MeshMessage::PqExchange { mlkem_ciphertext } => {
                assert_eq!(mlkem_ciphertext, "dGVzdA==");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_wire_frame_encode_decode() {
        let payload = b"hello mesh";
        let frame = encode_frame(payload);
        assert_eq!(frame.len(), 4 + payload.len());

        let mut header = [0u8; 4];
        header.copy_from_slice(&frame[..4]);
        let length = decode_frame_length(&header);
        assert_eq!(length as usize, payload.len());
        assert_eq!(&frame[4..], payload);
    }

    #[test]
    fn test_wire_frame_empty_payload() {
        let frame = encode_frame(b"");
        let mut header = [0u8; 4];
        header.copy_from_slice(&frame[..4]);
        assert_eq!(decode_frame_length(&header), 0);
    }
}
