use std::sync::Arc;

use anyhow::Result;
use snow::TransportState;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
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
///
/// The TcpStream is split into separate read/write halves so that recv_loop
/// can hold the reader without blocking concurrent sends. The Noise TransportState
/// is protected by its own mutex since both encryption and decryption use the
/// same stateful object (counters, etc.).
pub struct MeshConnection {
    pub peer_id: String,
    pub transport_encrypt: Arc<Mutex<TransportState>>,
    pub transport_decrypt: Arc<Mutex<TransportState>>,
    pub reader: Arc<Mutex<OwnedReadHalf>>,
    pub writer: Arc<Mutex<OwnedWriteHalf>>,
    pub phase: TransportPhase,
    pub pq_rekey_material: [u8; 32],
}

impl MeshConnection {
    /// Create a MeshConnection from a completed handshake.
    /// The TcpStream is split into owned halves to allow concurrent send/recv.
    pub fn new(
        peer_id: String,
        stream: TcpStream,
        transport: TransportState,
        pq_rekey_material: [u8; 32],
    ) -> Self {
        // Split the stream so reads and writes can proceed concurrently
        let (reader, writer) = stream.into_split();
        // Clone the transport for separate encrypt/decrypt locks.
        // snow's TransportState doesn't impl Clone, so we use two separate states
        // by splitting on construction — instead, we share a single state under
        // a single mutex since Noise counters must be sequential anyway.
        // We use two Arc<Mutex<_>> pointing to the *same* logical slot via a wrapper.
        // Actually snow requires a single TransportState; we protect it with one mutex
        // used by both send and recv. This is correct — Noise nonces must be sequential.
        // The deadlock was stream vs stream, not transport vs transport.
        let transport = Arc::new(Mutex::new(transport));
        Self {
            peer_id,
            transport_encrypt: transport.clone(),
            transport_decrypt: transport,
            reader: Arc::new(Mutex::new(reader)),
            writer: Arc::new(Mutex::new(writer)),
            phase: TransportPhase::Connected,
            pq_rekey_material,
        }
    }

    /// Send a MeshMessage over the encrypted connection.
    /// Encrypts under transport lock, then writes under writer lock.
    pub async fn send_message(&self, msg: &MeshMessage) -> Result<()> {
        let json = serde_json::to_string(msg)?;
        // Encrypt first (brief lock on transport state)
        let frame = {
            let mut transport = self.transport_encrypt.lock().await;
            let mut encrypted = vec![0u8; json.len() + 16]; // AEAD tag overhead
            let len = transport.write_message(json.as_bytes(), &mut encrypted)?;
            messages::encode_frame(&encrypted[..len])
        };
        // Write to the dedicated write half (independent from reader)
        let mut writer = self.writer.lock().await;
        writer.write_all(&frame).await?;
        writer.flush().await?;
        Ok(())
    }

    /// Receive a MeshMessage from the encrypted connection.
    /// Reads under reader lock, then decrypts under transport lock.
    pub async fn recv_message(&self) -> Result<MeshMessage> {
        // Read the length-prefixed frame (reader lock only — does NOT block sends)
        let payload = {
            let mut reader = self.reader.lock().await;
            let mut header = [0u8; 4];
            reader.read_exact(&mut header).await?;
            let length = messages::decode_frame_length(&header);
            if length > messages::MAX_MESSAGE_SIZE {
                anyhow::bail!(
                    "frame too large: {} bytes (max {})",
                    length,
                    messages::MAX_MESSAGE_SIZE
                );
            }
            let mut payload = vec![0u8; length as usize];
            reader.read_exact(&mut payload).await?;
            payload
        };
        // Decrypt (brief lock on transport state)
        let mut transport = self.transport_decrypt.lock().await;
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
