use std::sync::Arc;

use anyhow::Result;
use snow::TransportState;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::messages::{self, MeshMessage};

/// Connection state machine.
#[derive(Debug, Clone, PartialEq)]
pub enum TransportPhase {
    Connecting,
    Handshaking,
    PqExchange,
    Connected,
    Failed(String),
}

/// An active encrypted connection to a peer.
pub struct MeshConnection {
    pub peer_id: String,
    pub transport: Arc<Mutex<TransportState>>,
    pub stream: Arc<Mutex<TcpStream>>,
    pub phase: TransportPhase,
    pub pq_rekey_material: [u8; 32],
}

impl MeshConnection {
    /// Send a MeshMessage over the encrypted connection.
    pub async fn send_message(&self, msg: &MeshMessage) -> Result<()> {
        let json = serde_json::to_string(msg)?;
        let mut transport = self.transport.lock().await;
        let mut encrypted = vec![0u8; json.len() + 16]; // AEAD tag overhead
        let len = transport.write_message(json.as_bytes(), &mut encrypted)?;
        drop(transport);

        let frame = messages::encode_frame(&encrypted[..len]);
        let mut stream = self.stream.lock().await;
        stream.write_all(&frame).await?;
        stream.flush().await?;
        Ok(())
    }

    /// Receive a MeshMessage from the encrypted connection.
    pub async fn recv_message(&self) -> Result<MeshMessage> {
        let mut stream = self.stream.lock().await;
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).await?;
        let length = messages::decode_frame_length(&header);
        if length > messages::MAX_MESSAGE_SIZE {
            anyhow::bail!(
                "frame too large: {} bytes (max {})",
                length,
                messages::MAX_MESSAGE_SIZE
            );
        }
        let mut payload = vec![0u8; length as usize];
        stream.read_exact(&mut payload).await?;
        drop(stream);

        let mut transport = self.transport.lock().await;
        let mut decrypted = vec![0u8; payload.len()];
        let len = transport.read_message(&payload, &mut decrypted)?;
        drop(transport);

        let msg: MeshMessage = serde_json::from_slice(&decrypted[..len])?;
        Ok(msg)
    }
}

/// Exponential backoff parameters for reconnection.
pub struct ReconnectBackoff {
    pub attempt: u32,
    pub base_secs: u64,
    pub max_secs: u64,
}

impl ReconnectBackoff {
    pub fn new() -> Self {
        Self {
            attempt: 0,
            base_secs: 1,
            max_secs: 60,
        }
    }

    /// Get the delay for the current attempt and advance to the next.
    pub fn next_delay(&mut self) -> std::time::Duration {
        let delay = std::cmp::min(
            self.base_secs * 2u64.saturating_pow(self.attempt),
            self.max_secs,
        );
        self.attempt = self.attempt.saturating_add(1);
        std::time::Duration::from_secs(delay)
    }

    /// Reset the backoff after a successful connection.
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

/// Background heartbeat sender loop. Sends heartbeat messages at the given interval.
pub async fn heartbeat_loop(
    connection: Arc<MeshConnection>,
    interval_secs: u64,
    cancel: CancellationToken,
) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(interval_secs));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::debug!(peer_id = %connection.peer_id, "heartbeat loop stopped");
                break;
            }
            _ = interval.tick() => {
                let msg = MeshMessage::Heartbeat {
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                if let Err(e) = connection.send_message(&msg).await {
                    tracing::warn!(
                        peer_id = %connection.peer_id,
                        error = %e,
                        "heartbeat send failed"
                    );
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconnect_backoff() {
        let mut backoff = ReconnectBackoff::new();
        assert_eq!(backoff.next_delay(), std::time::Duration::from_secs(1));
        assert_eq!(backoff.next_delay(), std::time::Duration::from_secs(2));
        assert_eq!(backoff.next_delay(), std::time::Duration::from_secs(4));
        assert_eq!(backoff.next_delay(), std::time::Duration::from_secs(8));
        assert_eq!(backoff.next_delay(), std::time::Duration::from_secs(16));
        assert_eq!(backoff.next_delay(), std::time::Duration::from_secs(32));
        assert_eq!(backoff.next_delay(), std::time::Duration::from_secs(60)); // capped
        assert_eq!(backoff.next_delay(), std::time::Duration::from_secs(60)); // stays capped
    }

    #[test]
    fn test_reconnect_backoff_reset() {
        let mut backoff = ReconnectBackoff::new();
        let _ = backoff.next_delay();
        let _ = backoff.next_delay();
        backoff.reset();
        assert_eq!(backoff.next_delay(), std::time::Duration::from_secs(1));
    }
}
