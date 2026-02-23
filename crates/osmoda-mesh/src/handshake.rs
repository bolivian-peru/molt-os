use anyhow::{Context, Result};
use base64::Engine;
use hkdf::Hkdf;
use sha2::Sha256;
use snow::TransportState;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::identity::{MeshIdentity, MeshPublicIdentity};
use crate::messages::{self, MeshMessage};

/// HKDF info string for hybrid key derivation.
const HKDF_INFO: &[u8] = b"osMODA-mesh-v1";

/// Result of a completed handshake.
pub struct HandshakeResult {
    pub transport: TransportState,
    pub peer_identity: MeshPublicIdentity,
    /// Hybrid PQ re-key material (32 bytes) derived from HKDF over both Noise and ML-KEM shared secrets.
    pub pq_rekey_material: [u8; 32],
}

/// Perform the Noise_XX handshake as the initiator (connecting to a peer).
pub async fn initiate_handshake(
    stream: &mut TcpStream,
    identity: &MeshIdentity,
) -> Result<HandshakeResult> {
    let mut handshake = snow::Builder::new("Noise_XX_25519_ChaChaPoly_BLAKE2s".parse()?)
        .local_private_key(&identity.noise_static_keypair.private)
        .build_initiator()?;

    let mut buf = vec![0u8; 65535];

    // Message 1: Initiator → Responder (e)
    let len = handshake.write_message(&[], &mut buf)?;
    send_frame(stream, &buf[..len]).await?;

    // Message 2: Responder → Initiator (e, ee, s, es)
    let msg2 = recv_frame(stream).await?;
    let _len = handshake.read_message(&msg2, &mut buf)?;

    // Message 3: Initiator → Responder (s, se)
    let len = handshake.write_message(&[], &mut buf)?;
    send_frame(stream, &buf[..len]).await?;

    // Save handshake hash before entering transport mode (only available on HandshakeState)
    let handshake_hash = handshake.get_handshake_hash().to_vec();

    // Enter transport mode
    let mut transport = handshake.into_transport_mode()?;

    // Exchange identities as first application message
    let my_identity_json = serde_json::to_string(&identity.public_identity)?;
    let mut encrypted = vec![0u8; my_identity_json.len() + 16]; // AEAD tag overhead
    let len = transport.write_message(my_identity_json.as_bytes(), &mut encrypted)?;
    send_frame(stream, &encrypted[..len]).await?;

    let peer_frame = recv_frame(stream).await?;
    let mut decrypted = vec![0u8; peer_frame.len()];
    let len = transport.read_message(&peer_frame, &mut decrypted)?;
    let peer_identity: MeshPublicIdentity = serde_json::from_slice(&decrypted[..len])
        .context("failed to parse peer identity")?;

    // Verify peer identity signature
    if !MeshIdentity::verify_identity(&peer_identity)? {
        anyhow::bail!("peer identity signature verification failed");
    }

    // ML-KEM PQ exchange (inside the encrypted tunnel)
    // Initiator encapsulates to responder's EK
    let (ct_to_peer, ss_initiator) =
        MeshIdentity::mlkem_encapsulate(&peer_identity.mlkem_encap_key)?;
    let ct_msg = MeshMessage::PqExchange {
        mlkem_ciphertext: base64::engine::general_purpose::STANDARD.encode(&ct_to_peer),
    };
    let ct_json = serde_json::to_string(&ct_msg)?;
    let mut enc_ct = vec![0u8; ct_json.len() + 16];
    let len = transport.write_message(ct_json.as_bytes(), &mut enc_ct)?;
    send_frame(stream, &enc_ct[..len]).await?;

    // Receive responder's encapsulation to our EK
    let peer_ct_frame = recv_frame(stream).await?;
    let mut dec_ct = vec![0u8; peer_ct_frame.len()];
    let len = transport.read_message(&peer_ct_frame, &mut dec_ct)?;
    let peer_ct_msg: MeshMessage = serde_json::from_slice(&dec_ct[..len])?;
    let ss_responder = match peer_ct_msg {
        MeshMessage::PqExchange { mlkem_ciphertext } => {
            let ct_bytes = base64::engine::general_purpose::STANDARD.decode(&mlkem_ciphertext)?;
            identity.mlkem_decapsulate(&ct_bytes)?
        }
        _ => anyhow::bail!("expected PqExchange message from responder"),
    };

    // Hybrid re-key: HKDF-SHA256 over Noise handshake hash + both ML-KEM shared secrets
    let pq_rekey_material = derive_hybrid_key(&handshake_hash, &ss_initiator, &ss_responder)?;

    Ok(HandshakeResult {
        transport,
        peer_identity,
        pq_rekey_material,
    })
}

/// Perform the Noise_XX handshake as the responder (accepting a connection).
pub async fn respond_handshake(
    stream: &mut TcpStream,
    identity: &MeshIdentity,
) -> Result<HandshakeResult> {
    let mut handshake = snow::Builder::new("Noise_XX_25519_ChaChaPoly_BLAKE2s".parse()?)
        .local_private_key(&identity.noise_static_keypair.private)
        .build_responder()?;

    let mut buf = vec![0u8; 65535];

    // Message 1: Initiator → Responder (e)
    let msg1 = recv_frame(stream).await?;
    let _len = handshake.read_message(&msg1, &mut buf)?;

    // Message 2: Responder → Initiator (e, ee, s, es)
    let len = handshake.write_message(&[], &mut buf)?;
    send_frame(stream, &buf[..len]).await?;

    // Message 3: Initiator → Responder (s, se)
    let msg3 = recv_frame(stream).await?;
    let _len = handshake.read_message(&msg3, &mut buf)?;

    // Save handshake hash before entering transport mode
    let handshake_hash = handshake.get_handshake_hash().to_vec();

    // Enter transport mode
    let mut transport = handshake.into_transport_mode()?;

    // Exchange identities
    let peer_frame = recv_frame(stream).await?;
    let mut decrypted = vec![0u8; peer_frame.len()];
    let len = transport.read_message(&peer_frame, &mut decrypted)?;
    let peer_identity: MeshPublicIdentity = serde_json::from_slice(&decrypted[..len])
        .context("failed to parse peer identity")?;

    if !MeshIdentity::verify_identity(&peer_identity)? {
        anyhow::bail!("peer identity signature verification failed");
    }

    let my_identity_json = serde_json::to_string(&identity.public_identity)?;
    let mut encrypted = vec![0u8; my_identity_json.len() + 16];
    let len = transport.write_message(my_identity_json.as_bytes(), &mut encrypted)?;
    send_frame(stream, &encrypted[..len]).await?;

    // ML-KEM PQ exchange
    // Receive initiator's encapsulation to our EK
    let peer_ct_frame = recv_frame(stream).await?;
    let mut dec_ct = vec![0u8; peer_ct_frame.len()];
    let len = transport.read_message(&peer_ct_frame, &mut dec_ct)?;
    let peer_ct_msg: MeshMessage = serde_json::from_slice(&dec_ct[..len])?;
    let ss_initiator = match peer_ct_msg {
        MeshMessage::PqExchange { mlkem_ciphertext } => {
            let ct_bytes = base64::engine::general_purpose::STANDARD.decode(&mlkem_ciphertext)?;
            identity.mlkem_decapsulate(&ct_bytes)?
        }
        _ => anyhow::bail!("expected PqExchange message from initiator"),
    };

    // Responder encapsulates to initiator's EK
    let (ct_to_peer, ss_responder) =
        MeshIdentity::mlkem_encapsulate(&peer_identity.mlkem_encap_key)?;
    let ct_msg = MeshMessage::PqExchange {
        mlkem_ciphertext: base64::engine::general_purpose::STANDARD.encode(&ct_to_peer),
    };
    let ct_json = serde_json::to_string(&ct_msg)?;
    let mut enc_ct = vec![0u8; ct_json.len() + 16];
    let len = transport.write_message(ct_json.as_bytes(), &mut enc_ct)?;
    send_frame(stream, &enc_ct[..len]).await?;

    // Hybrid re-key
    let pq_rekey_material = derive_hybrid_key(&handshake_hash, &ss_initiator, &ss_responder)?;

    Ok(HandshakeResult {
        transport,
        peer_identity,
        pq_rekey_material,
    })
}

/// Derive hybrid re-key material from Noise handshake hash + both ML-KEM shared secrets.
fn derive_hybrid_key(handshake_hash: &[u8], ss1: &[u8], ss2: &[u8]) -> Result<[u8; 32]> {
    // IKM = handshake_hash || mlkem_shared_1 || mlkem_shared_2
    let mut ikm = Vec::with_capacity(handshake_hash.len() + ss1.len() + ss2.len());
    ikm.extend_from_slice(handshake_hash);
    ikm.extend_from_slice(ss1);
    ikm.extend_from_slice(ss2);

    let hk = Hkdf::<Sha256>::new(None, &ikm);
    let mut output = [0u8; 32];
    hk.expand(HKDF_INFO, &mut output)
        .map_err(|e| anyhow::anyhow!("HKDF expand failed: {e}"))?;
    Ok(output)
}

/// Send a length-prefixed frame over TCP.
async fn send_frame(stream: &mut TcpStream, data: &[u8]) -> Result<()> {
    let frame = messages::encode_frame(data);
    stream.write_all(&frame).await?;
    stream.flush().await?;
    Ok(())
}

/// Receive a length-prefixed frame from TCP.
async fn recv_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
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
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn test_noise_xx_handshake_roundtrip() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let identity_a = MeshIdentity::load_or_create(dir_a.path()).unwrap();
        let identity_b = MeshIdentity::load_or_create(dir_b.path()).unwrap();

        // Save instance_id before moving identity_b into the spawned task
        let identity_b_instance_id = identity_b.public_identity.instance_id.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            respond_handshake(&mut stream, &identity_b).await.unwrap()
        });

        let mut client_stream = TcpStream::connect(addr).await.unwrap();
        let client_result = initiate_handshake(&mut client_stream, &identity_a)
            .await
            .unwrap();

        let server_result = server.await.unwrap();

        // Both sides should see the correct peer identity
        assert_eq!(
            client_result.peer_identity.instance_id,
            identity_b_instance_id
        );

        // PQ rekey material should match
        assert_eq!(
            client_result.pq_rekey_material, server_result.pq_rekey_material
        );
    }

    #[tokio::test]
    async fn test_transport_encrypt_decrypt() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let identity_a = MeshIdentity::load_or_create(dir_a.path()).unwrap();
        let identity_b = MeshIdentity::load_or_create(dir_b.path()).unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut result = respond_handshake(&mut stream, &identity_b).await.unwrap();

            // Receive a message
            let frame = recv_frame(&mut stream).await.unwrap();
            let mut plaintext = vec![0u8; frame.len()];
            let len = result.transport.read_message(&frame, &mut plaintext).unwrap();
            String::from_utf8(plaintext[..len].to_vec()).unwrap()
        });

        let mut client_stream = TcpStream::connect(addr).await.unwrap();
        let mut client_result = initiate_handshake(&mut client_stream, &identity_a)
            .await
            .unwrap();

        // Send a message
        let msg = b"hello encrypted world";
        let mut encrypted = vec![0u8; msg.len() + 16];
        let len = client_result
            .transport
            .write_message(msg, &mut encrypted)
            .unwrap();
        send_frame(&mut client_stream, &encrypted[..len])
            .await
            .unwrap();

        let received = server.await.unwrap();
        assert_eq!(received, "hello encrypted world");
    }

    #[test]
    fn test_derive_hybrid_key() {
        let hh = [1u8; 32];
        let ss1 = [2u8; 32];
        let ss2 = [3u8; 32];

        let key1 = derive_hybrid_key(&hh, &ss1, &ss2).unwrap();
        let key2 = derive_hybrid_key(&hh, &ss1, &ss2).unwrap();
        assert_eq!(key1, key2, "same inputs should produce same key");

        let key3 = derive_hybrid_key(&hh, &ss2, &ss1).unwrap();
        assert_ne!(key1, key3, "different input order should produce different key");
    }
}
