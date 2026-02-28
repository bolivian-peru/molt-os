use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Solana transaction parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolTxParams {
    /// Recipient public key (base58-encoded).
    pub to: String,
    /// Amount in lamports (1 SOL = 1_000_000_000 lamports).
    pub lamports: u64,
    /// Recent blockhash (base58-encoded, must be provided by caller).
    pub recent_blockhash: String,
}

/// Result of building a Solana transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolTxResult {
    /// Base58-encoded signed transaction.
    pub signed_tx: String,
    /// Transaction signature (first signature, base58).
    pub signature: String,
    /// Sender public key (base58).
    pub from: String,
    /// Recipient public key (base58).
    pub to: String,
    /// Amount in lamports.
    pub lamports: u64,
}

/// System Program ID (all zeros except last byte = 0)
const SYSTEM_PROGRAM_ID: [u8; 32] = [0u8; 32];

/// System Program transfer instruction index.
const TRANSFER_INSTRUCTION_INDEX: u32 = 2;

/// Build and sign a Solana transfer transaction.
///
/// This builds a raw System Program transfer instruction and signs it.
/// Returns base58-encoded signed transaction ready for `sendTransaction` RPC.
pub fn build_and_sign_transfer(
    key_bytes: &[u8],
    params: &SolTxParams,
) -> Result<SolTxResult> {
    use ed25519_dalek::{Signer, SigningKey};

    if key_bytes.len() < 32 {
        anyhow::bail!("invalid SOL key: expected 32 bytes, got {}", key_bytes.len());
    }

    let mut secret = [0u8; 32];
    secret.copy_from_slice(&key_bytes[..32]);
    let signing_key = SigningKey::from_bytes(&secret);
    let from_pubkey = signing_key.verifying_key();
    let from_bytes = from_pubkey.to_bytes();

    // Decode recipient pubkey
    let to_bytes = bs58::decode(&params.to)
        .into_vec()
        .context("invalid recipient address (must be base58)")?;
    if to_bytes.len() != 32 {
        anyhow::bail!("recipient pubkey must be 32 bytes, got {}", to_bytes.len());
    }
    let mut to_key = [0u8; 32];
    to_key.copy_from_slice(&to_bytes);

    // Decode recent blockhash
    let blockhash_bytes = bs58::decode(&params.recent_blockhash)
        .into_vec()
        .context("invalid recent_blockhash (must be base58)")?;
    if blockhash_bytes.len() != 32 {
        anyhow::bail!("recent_blockhash must be 32 bytes, got {}", blockhash_bytes.len());
    }
    let mut blockhash = [0u8; 32];
    blockhash.copy_from_slice(&blockhash_bytes);

    // Build the Solana transaction message (legacy format)
    let message = build_transfer_message(
        &from_bytes,
        &to_key,
        params.lamports,
        &blockhash,
    );

    // Sign the message
    let signature = signing_key.sign(&message);
    let sig_bytes = signature.to_bytes();

    // Build the full signed transaction:
    // [compact-u16: num_signatures][signature(s)][message]
    let mut tx = Vec::new();

    // Number of signatures (compact-u16 encoding, 1 = 0x01)
    tx.push(1u8);

    // Signature (64 bytes)
    tx.extend_from_slice(&sig_bytes);

    // Message
    tx.extend_from_slice(&message);

    // Zero the secret
    zeroize::Zeroize::zeroize(&mut secret);

    Ok(SolTxResult {
        signed_tx: bs58::encode(&tx).into_string(),
        signature: bs58::encode(&sig_bytes).into_string(),
        from: bs58::encode(&from_bytes).into_string(),
        to: params.to.clone(),
        lamports: params.lamports,
    })
}

/// Build a Solana transaction message for a System Program transfer.
///
/// Legacy message format:
/// - header: [num_required_signatures, num_readonly_signed, num_readonly_unsigned]
/// - account_keys: [from, to, system_program]
/// - recent_blockhash: [32 bytes]
/// - instructions: [compact-u16 count, instruction...]
fn build_transfer_message(
    from: &[u8; 32],
    to: &[u8; 32],
    lamports: u64,
    recent_blockhash: &[u8; 32],
) -> Vec<u8> {
    let mut msg = Vec::new();

    // Header
    msg.push(1); // num_required_signatures
    msg.push(0); // num_readonly_signed_accounts
    msg.push(1); // num_readonly_unsigned_accounts (system program)

    // Account keys (3 accounts: from, to, system_program)
    let num_accounts = 3u8;
    msg.push(num_accounts); // compact-u16 encoding for small numbers is just 1 byte
    msg.extend_from_slice(from);
    msg.extend_from_slice(to);
    msg.extend_from_slice(&SYSTEM_PROGRAM_ID);

    // Recent blockhash
    msg.extend_from_slice(recent_blockhash);

    // Instructions (1 instruction)
    msg.push(1u8); // compact-u16: 1 instruction

    // System Program Transfer instruction:
    // - program_id_index: 2 (system program is 3rd account, 0-indexed)
    // - accounts: [0 (from, signer+writable), 1 (to, writable)]
    // - data: [u32 LE instruction index (2), u64 LE lamports]
    msg.push(2u8); // program_id_index

    // Account indices (compact-u16 length + indices)
    msg.push(2u8); // 2 accounts
    msg.push(0u8); // from (index 0)
    msg.push(1u8); // to (index 1)

    // Instruction data: u32 LE (transfer = 2) + u64 LE (lamports)
    let mut data = Vec::new();
    data.extend_from_slice(&TRANSFER_INSTRUCTION_INDEX.to_le_bytes());
    data.extend_from_slice(&lamports.to_le_bytes());

    // Data length (compact-u16)
    msg.push(data.len() as u8); // 12 bytes fits in 1 byte compact-u16
    msg.extend_from_slice(&data);

    msg
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_keypair() -> Vec<u8> {
        use ed25519_dalek::SigningKey;
        let mut secret = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut secret);
        let signing_key = SigningKey::from_bytes(&secret);
        let pubkey = signing_key.verifying_key();
        // Return just the 32 secret bytes
        let _ = pubkey; // suppress unused warning
        secret.to_vec()
    }

    fn fake_blockhash() -> String {
        // A valid base58-encoded 32-byte hash
        bs58::encode([1u8; 32]).into_string()
    }

    fn fake_pubkey() -> String {
        bs58::encode([2u8; 32]).into_string()
    }

    #[test]
    fn test_build_transfer_basic() {
        let key_bytes = test_keypair();
        let params = SolTxParams {
            to: fake_pubkey(),
            lamports: 1_000_000_000, // 1 SOL
            recent_blockhash: fake_blockhash(),
        };

        let result = build_and_sign_transfer(&key_bytes, &params).unwrap();

        // Signed tx should be non-empty base58
        assert!(!result.signed_tx.is_empty());
        // Signature should be base58
        assert!(!result.signature.is_empty());
        // From should be valid base58 pubkey
        assert!(!result.from.is_empty());
        assert_eq!(result.lamports, 1_000_000_000);
    }

    #[test]
    fn test_transfer_message_structure() {
        let from = [1u8; 32];
        let to = [2u8; 32];
        let blockhash = [3u8; 32];

        let msg = build_transfer_message(&from, &to, 42, &blockhash);

        // Header: 3 bytes
        assert_eq!(msg[0], 1); // num_required_signatures
        assert_eq!(msg[1], 0); // num_readonly_signed
        assert_eq!(msg[2], 1); // num_readonly_unsigned

        // Num accounts
        assert_eq!(msg[3], 3);

        // Account keys: 3 * 32 = 96 bytes starting at offset 4
        assert_eq!(&msg[4..36], &from);
        assert_eq!(&msg[36..68], &to);
        assert_eq!(&msg[68..100], &SYSTEM_PROGRAM_ID);

        // Recent blockhash at offset 100
        assert_eq!(&msg[100..132], &blockhash);
    }

    #[test]
    fn test_signature_verification() {
        use ed25519_dalek::{Signature, Verifier, SigningKey};

        let mut secret = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut secret);
        let signing_key = SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();

        let params = SolTxParams {
            to: fake_pubkey(),
            lamports: 500_000,
            recent_blockhash: fake_blockhash(),
        };

        let result = build_and_sign_transfer(&secret.to_vec(), &params).unwrap();

        // Decode the signature and verify it against the message
        let sig_bytes = bs58::decode(&result.signature).into_vec().unwrap();
        let sig = Signature::from_slice(&sig_bytes).unwrap();

        let tx_bytes = bs58::decode(&result.signed_tx).into_vec().unwrap();
        // Message starts after 1 byte (num sigs) + 64 bytes (signature)
        let message = &tx_bytes[65..];

        assert!(verifying_key.verify(message, &sig).is_ok());
    }

    #[test]
    fn test_invalid_key_length() {
        let short_key = vec![0u8; 16];
        let params = SolTxParams {
            to: fake_pubkey(),
            lamports: 100,
            recent_blockhash: fake_blockhash(),
        };

        assert!(build_and_sign_transfer(&short_key, &params).is_err());
    }

    #[test]
    fn test_invalid_recipient() {
        let key_bytes = test_keypair();
        let params = SolTxParams {
            to: "invalid!".to_string(),
            lamports: 100,
            recent_blockhash: fake_blockhash(),
        };

        assert!(build_and_sign_transfer(&key_bytes, &params).is_err());
    }
}
