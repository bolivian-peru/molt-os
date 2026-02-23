use serde::{Deserialize, Serialize};

use crate::identity::MeshPublicIdentity;

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
    },
    LedgerSync {
        events: Vec<serde_json::Value>,
        since: String,
    },
    Command {
        id: String,
        command: String,
        args: serde_json::Value,
    },
    CommandResponse {
        command_id: String,
        status: String,
        result: serde_json::Value,
    },
    PeerAnnounce {
        identity: MeshPublicIdentity,
    },
    KeyRotation {
        new_noise_pubkey: String,
        new_mlkem_ek: String,
        signature: String,
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
    fn test_serde_roundtrip_chat() {
        let msg = MeshMessage::Chat {
            from: "admin".to_string(),
            text: "hello peer".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: MeshMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            MeshMessage::Chat { from, text } => {
                assert_eq!(from, "admin");
                assert_eq!(text, "hello peer");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_serde_roundtrip_command() {
        let msg = MeshMessage::Command {
            id: "cmd-1".to_string(),
            command: "health_check".to_string(),
            args: serde_json::json!({"verbose": true}),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: MeshMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            MeshMessage::Command { id, command, args } => {
                assert_eq!(id, "cmd-1");
                assert_eq!(command, "health_check");
                assert_eq!(args["verbose"], true);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_serde_roundtrip_command_response() {
        let msg = MeshMessage::CommandResponse {
            command_id: "cmd-1".to_string(),
            status: "ok".to_string(),
            result: serde_json::json!({"output": "all good"}),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: MeshMessage = serde_json::from_str(&json).unwrap();
        match decoded {
            MeshMessage::CommandResponse {
                command_id,
                status,
                result,
            } => {
                assert_eq!(command_id, "cmd-1");
                assert_eq!(status, "ok");
                assert_eq!(result["output"], "all good");
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
