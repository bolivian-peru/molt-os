use std::sync::Arc;

use anyhow::Result;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::ChaCha20Poly1305;
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
    /// Counter-based nonce for PQ encryption (send direction).
    pq_encrypt_nonce: Arc<Mutex<u64>>,
    /// Counter-based nonce for PQ decryption (recv direction).
    pq_decrypt_nonce: Arc<Mutex<u64>>,
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
            pq_encrypt_nonce: Arc::new(Mutex::new(0)),
            pq_decrypt_nonce: Arc::new(Mutex::new(0)),
        }
    }

    /// Send a MeshMessage over the encrypted connection.
    /// Double-layer encryption: Noise (X25519) then ChaCha20-Poly1305 (ML-KEM material).
    /// This ensures post-quantum protection is real — decryption requires both
    /// the Noise session keys AND the ML-KEM-derived PQ key material.
    pub async fn send_message(&self, msg: &MeshMessage) -> Result<()> {
        let json = serde_json::to_string(msg)?;

        // Layer 1: Noise encrypt (classical X25519 protection)
        let noise_ciphertext = {
            let mut transport = self.transport_encrypt.lock().await;
            let mut encrypted = vec![0u8; json.len() + 16]; // AEAD tag overhead
            let len = transport.write_message(json.as_bytes(), &mut encrypted)?;
            encrypted.truncate(len);
            encrypted
        };

        // Layer 2: PQ encrypt (ChaCha20-Poly1305 keyed by ML-KEM material)
        let pq_ciphertext = {
            let mut counter = self.pq_encrypt_nonce.lock().await;
            let nonce = make_pq_nonce(*counter);
            *counter += 1;
            let cipher = ChaCha20Poly1305::new_from_slice(&self.pq_rekey_material)
                .map_err(|e| anyhow::anyhow!("PQ cipher init failed: {e}"))?;
            cipher
                .encrypt(&nonce, noise_ciphertext.as_slice())
                .map_err(|e| anyhow::anyhow!("PQ encrypt failed: {e}"))?
        };

        // Frame and send
        let frame = messages::encode_frame(&pq_ciphertext);
        let mut writer = self.writer.lock().await;
        writer.write_all(&frame).await?;
        writer.flush().await?;
        Ok(())
    }

    /// Receive a MeshMessage from the encrypted connection.
    /// Double-layer decryption: ChaCha20-Poly1305 (PQ) then Noise (classical).
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

        // Layer 2 (unwrap): PQ decrypt (ChaCha20-Poly1305 keyed by ML-KEM material)
        let noise_ciphertext = {
            let mut counter = self.pq_decrypt_nonce.lock().await;
            let nonce = make_pq_nonce(*counter);
            *counter += 1;
            let cipher = ChaCha20Poly1305::new_from_slice(&self.pq_rekey_material)
                .map_err(|e| anyhow::anyhow!("PQ cipher init failed: {e}"))?;
            cipher
                .decrypt(&nonce, payload.as_slice())
                .map_err(|e| anyhow::anyhow!("PQ decrypt failed: {e}"))?
        };

        // Layer 1 (unwrap): Noise decrypt (classical X25519 protection)
        let mut transport = self.transport_decrypt.lock().await;
        let mut decrypted = vec![0u8; noise_ciphertext.len()];
        let len = transport.read_message(&noise_ciphertext, &mut decrypted)?;
        drop(transport);

        let msg: MeshMessage = serde_json::from_slice(&decrypted[..len])?;
        Ok(msg)
    }
}

/// Build a 96-bit nonce from a counter for the PQ ChaCha20-Poly1305 layer.
/// The counter occupies the first 8 bytes (little-endian u64), last 4 bytes are zero.
fn make_pq_nonce(counter: u64) -> chacha20poly1305::Nonce {
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes[..8].copy_from_slice(&counter.to_le_bytes());
    *chacha20poly1305::Nonce::from_slice(&nonce_bytes)
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

    #[test]
    fn test_pq_nonce_generation() {
        let n0 = make_pq_nonce(0);
        let n1 = make_pq_nonce(1);
        assert_ne!(n0, n1, "different counters should produce different nonces");

        // Counter 0 → first 8 bytes all zero, last 4 zero
        let expected_zero: [u8; 12] = [0; 12];
        assert_eq!(n0.as_slice(), &expected_zero);

        // Counter 1 → first byte is 1 (LE), rest zero
        let mut expected_one = [0u8; 12];
        expected_one[0] = 1;
        assert_eq!(n1.as_slice(), &expected_one);
    }

    /// Full end-to-end test: Noise handshake → MeshConnection with PQ double-layer
    /// encryption. Verifies that messages survive Noise + ChaCha20-Poly1305(ML-KEM)
    /// and that both sides can communicate bidirectionally.
    #[tokio::test]
    async fn test_mesh_connection_pq_hybrid_roundtrip() {
        use crate::handshake::{initiate_handshake, respond_handshake};
        use crate::identity::MeshIdentity;
        use tokio::net::TcpListener;

        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let identity_a = MeshIdentity::load_or_create(dir_a.path()).unwrap();
        let identity_b = MeshIdentity::load_or_create(dir_b.path()).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let result = respond_handshake(&mut stream, &identity_b).await.unwrap();
            MeshConnection::new(
                "client".to_string(),
                stream,
                result.transport,
                result.pq_rekey_material,
            )
        });

        let mut client_stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let client_result = initiate_handshake(&mut client_stream, &identity_a)
            .await
            .unwrap();
        let client_conn = Arc::new(MeshConnection::new(
            "server".to_string(),
            client_stream,
            client_result.transport,
            client_result.pq_rekey_material,
        ));

        let server_conn = Arc::new(server.await.unwrap());

        // Send from client → server (through Noise + PQ layers)
        let msg = MeshMessage::Chat {
            from: "test".to_string(),
            text: "hello PQ world".to_string(),
            room_id: None,
        };
        client_conn.send_message(&msg).await.unwrap();

        let received = server_conn.recv_message().await.unwrap();
        match received {
            MeshMessage::Chat { text, .. } => assert_eq!(text, "hello PQ world"),
            _ => panic!("wrong message type received"),
        }

        // Send from server → client (bidirectional)
        let reply = MeshMessage::Heartbeat {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        };
        server_conn.send_message(&reply).await.unwrap();

        let received_reply = client_conn.recv_message().await.unwrap();
        match received_reply {
            MeshMessage::Heartbeat { timestamp } => {
                assert_eq!(timestamp, "2026-01-01T00:00:00Z");
            }
            _ => panic!("wrong message type received"),
        }

        // Send multiple messages to verify nonce counter synchronization
        for i in 0..5 {
            let msg = MeshMessage::Chat {
                from: "test".to_string(),
                text: format!("message {i}"),
                room_id: None,
            };
            client_conn.send_message(&msg).await.unwrap();
            let received = server_conn.recv_message().await.unwrap();
            match received {
                MeshMessage::Chat { text, .. } => assert_eq!(text, format!("message {i}")),
                _ => panic!("wrong message type"),
            }
        }
    }

    /// Verify that mismatched PQ key material causes decryption failure.
    #[tokio::test]
    async fn test_pq_key_mismatch_fails() {
        use crate::handshake::{initiate_handshake, respond_handshake};
        use crate::identity::MeshIdentity;
        use tokio::net::TcpListener;

        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let identity_a = MeshIdentity::load_or_create(dir_a.path()).unwrap();
        let identity_b = MeshIdentity::load_or_create(dir_b.path()).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut result = respond_handshake(&mut stream, &identity_b).await.unwrap();
            // Tamper with PQ key material on server side
            result.pq_rekey_material = [0xAA; 32];
            MeshConnection::new(
                "client".to_string(),
                stream,
                result.transport,
                result.pq_rekey_material,
            )
        });

        let mut client_stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let client_result = initiate_handshake(&mut client_stream, &identity_a)
            .await
            .unwrap();
        let client_conn = Arc::new(MeshConnection::new(
            "server".to_string(),
            client_stream,
            client_result.transport,
            client_result.pq_rekey_material,
        ));

        let server_conn = Arc::new(server.await.unwrap());

        // Client sends a message — server should fail to decrypt because PQ keys don't match
        let msg = MeshMessage::Heartbeat {
            timestamp: "2026-01-01T00:00:00Z".to_string(),
        };
        client_conn.send_message(&msg).await.unwrap();

        let result = server_conn.recv_message().await;
        assert!(result.is_err(), "decryption should fail with mismatched PQ keys");
    }
}
