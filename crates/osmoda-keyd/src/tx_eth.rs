use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// EIP-1559 transaction parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EthTxParams {
    /// Chain ID (1 = mainnet, 11155111 = sepolia, etc.)
    pub chain_id: u64,
    /// Transaction nonce (must be provided by caller).
    pub nonce: u64,
    /// Recipient address (0x-prefixed hex, 20 bytes).
    pub to: String,
    /// Transfer value in wei (decimal string).
    pub value: String,
    /// Max fee per gas in wei.
    pub max_fee_per_gas: u64,
    /// Max priority fee per gas (tip) in wei.
    pub max_priority_fee_per_gas: u64,
    /// Gas limit.
    pub gas_limit: u64,
    /// Optional calldata (hex-encoded, no 0x prefix).
    pub data: Option<String>,
}

impl Default for EthTxParams {
    fn default() -> Self {
        Self {
            chain_id: 1,
            nonce: 0,
            to: String::new(),
            value: "0".to_string(),
            max_fee_per_gas: 30_000_000_000,       // 30 gwei
            max_priority_fee_per_gas: 1_000_000_000, // 1 gwei
            gas_limit: 21_000,                       // simple transfer
            data: None,
        }
    }
}

/// Result of building an Ethereum transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EthTxResult {
    pub signed_tx: String,
    pub tx_hash: String,
    pub from: String,
    pub to: String,
    pub value: String,
    pub chain_id: u64,
}

/// Build and sign an EIP-1559 transaction using RLP encoding.
///
/// Returns the signed transaction as hex (ready for eth_sendRawTransaction)
/// and the transaction hash.
pub fn build_and_sign_eip1559(
    key_bytes: &[u8],
    params: &EthTxParams,
) -> Result<EthTxResult> {
    use k256::ecdsa::{SigningKey, signature::hazmat::PrehashSigner};

    let signing_key = SigningKey::from_slice(key_bytes)
        .map_err(|e| anyhow::anyhow!("invalid ETH key: {e}"))?;
    let verifying_key = signing_key.verifying_key();

    // Derive sender address
    let pubkey_bytes = verifying_key.to_encoded_point(false);
    let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..];
    let hash = keccak256(pubkey_uncompressed);
    let from_addr = format!("0x{}", hex::encode(&hash[12..]));

    // Parse 'to' address
    let to_bytes = parse_address(&params.to)
        .context("invalid 'to' address")?;

    // Parse value (wei as decimal string)
    let value: u128 = params.value.parse()
        .context("invalid value — must be decimal wei string")?;

    // Parse optional calldata
    let data_bytes = match &params.data {
        Some(d) => hex::decode(d).context("invalid calldata hex")?,
        None => Vec::new(),
    };

    // RLP-encode the unsigned EIP-1559 transaction
    // EIP-1559 format: 0x02 || rlp([chain_id, nonce, max_priority_fee_per_gas, max_fee_per_gas, gas_limit, to, value, data, access_list])
    let unsigned_rlp = rlp_encode_eip1559_unsigned(
        params.chain_id,
        params.nonce,
        params.max_priority_fee_per_gas,
        params.max_fee_per_gas,
        params.gas_limit,
        &to_bytes,
        value,
        &data_bytes,
    );

    // Hash the unsigned tx for signing: keccak256(0x02 || unsigned_rlp)
    let mut signing_input = vec![0x02u8];
    signing_input.extend_from_slice(&unsigned_rlp);
    let msg_hash = keccak256(&signing_input);

    // Sign the hash
    let (sig, recid) = signing_key
        .sign_prehash(&msg_hash)
        .map_err(|e| anyhow::anyhow!("signing failed: {e}"))?;
    let sig_bytes = sig.to_bytes();
    let r = &sig_bytes[..32];
    let s = &sig_bytes[32..];
    let v = recid.to_byte();

    // RLP-encode the signed tx
    // 0x02 || rlp([chain_id, nonce, max_priority_fee_per_gas, max_fee_per_gas, gas_limit, to, value, data, access_list, v, r, s])
    let signed_rlp = rlp_encode_eip1559_signed(
        params.chain_id,
        params.nonce,
        params.max_priority_fee_per_gas,
        params.max_fee_per_gas,
        params.gas_limit,
        &to_bytes,
        value,
        &data_bytes,
        v,
        r,
        s,
    );

    let mut signed_tx = vec![0x02u8];
    signed_tx.extend_from_slice(&signed_rlp);
    let tx_hash = keccak256(&signed_tx);

    Ok(EthTxResult {
        signed_tx: format!("0x{}", hex::encode(&signed_tx)),
        tx_hash: format!("0x{}", hex::encode(tx_hash)),
        from: from_addr,
        to: params.to.clone(),
        value: params.value.clone(),
        chain_id: params.chain_id,
    })
}

/// Parse an Ethereum address (0x-prefixed hex) into 20 bytes.
fn parse_address(addr: &str) -> Result<[u8; 20]> {
    let hex_str = addr.strip_prefix("0x").unwrap_or(addr);
    if hex_str.len() != 40 {
        anyhow::bail!("address must be 40 hex chars, got {}", hex_str.len());
    }
    let bytes = hex::decode(hex_str)?;
    let mut out = [0u8; 20];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// Keccak-256 hash.
fn keccak256(data: &[u8]) -> [u8; 32] {
    use sha3::Digest;
    let mut hasher = sha3::Keccak256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

// ── Minimal RLP encoder ──

fn rlp_encode_u64(v: u64) -> Vec<u8> {
    if v == 0 {
        return vec![0x80]; // empty string
    }
    let bytes = v.to_be_bytes();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    rlp_encode_bytes(&bytes[start..])
}

fn rlp_encode_u128(v: u128) -> Vec<u8> {
    if v == 0 {
        return vec![0x80]; // empty string
    }
    let bytes = v.to_be_bytes();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(15);
    rlp_encode_bytes(&bytes[start..])
}

fn rlp_encode_bytes(data: &[u8]) -> Vec<u8> {
    if data.len() == 1 && data[0] < 0x80 {
        return vec![data[0]];
    }
    if data.is_empty() {
        return vec![0x80];
    }
    if data.len() <= 55 {
        let mut out = vec![0x80 + data.len() as u8];
        out.extend_from_slice(data);
        out
    } else {
        let len_bytes = encode_length_bytes(data.len());
        let mut out = vec![0xb7 + len_bytes.len() as u8];
        out.extend_from_slice(&len_bytes);
        out.extend_from_slice(data);
        out
    }
}

fn rlp_encode_list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut payload = Vec::new();
    for item in items {
        payload.extend_from_slice(item);
    }

    if payload.len() <= 55 {
        let mut out = vec![0xc0 + payload.len() as u8];
        out.extend_from_slice(&payload);
        out
    } else {
        let len_bytes = encode_length_bytes(payload.len());
        let mut out = vec![0xf7 + len_bytes.len() as u8];
        out.extend_from_slice(&len_bytes);
        out.extend_from_slice(&payload);
        out
    }
}

fn encode_length_bytes(len: usize) -> Vec<u8> {
    let bytes = (len as u64).to_be_bytes();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    bytes[start..].to_vec()
}

fn rlp_encode_eip1559_unsigned(
    chain_id: u64,
    nonce: u64,
    max_priority_fee: u64,
    max_fee: u64,
    gas_limit: u64,
    to: &[u8; 20],
    value: u128,
    data: &[u8],
) -> Vec<u8> {
    let items = vec![
        rlp_encode_u64(chain_id),
        rlp_encode_u64(nonce),
        rlp_encode_u64(max_priority_fee),
        rlp_encode_u64(max_fee),
        rlp_encode_u64(gas_limit),
        rlp_encode_bytes(to),
        rlp_encode_u128(value),
        rlp_encode_bytes(data),
        rlp_encode_list(&[]), // access_list (empty)
    ];
    rlp_encode_list(&items)
}

fn rlp_encode_eip1559_signed(
    chain_id: u64,
    nonce: u64,
    max_priority_fee: u64,
    max_fee: u64,
    gas_limit: u64,
    to: &[u8; 20],
    value: u128,
    data: &[u8],
    v: u8,
    r: &[u8],
    s: &[u8],
) -> Vec<u8> {
    // Strip leading zeros from r and s
    let r_stripped = strip_leading_zeros(r);
    let s_stripped = strip_leading_zeros(s);

    let items = vec![
        rlp_encode_u64(chain_id),
        rlp_encode_u64(nonce),
        rlp_encode_u64(max_priority_fee),
        rlp_encode_u64(max_fee),
        rlp_encode_u64(gas_limit),
        rlp_encode_bytes(to),
        rlp_encode_u128(value),
        rlp_encode_bytes(data),
        rlp_encode_list(&[]), // access_list
        rlp_encode_u64(v as u64),
        rlp_encode_bytes(r_stripped),
        rlp_encode_bytes(s_stripped),
    ];
    rlp_encode_list(&items)
}

fn strip_leading_zeros(data: &[u8]) -> &[u8] {
    let start = data.iter().position(|&b| b != 0).unwrap_or(data.len());
    if start == data.len() {
        &data[data.len() - 1..] // keep at least one byte
    } else {
        &data[start..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rlp_encode_u64() {
        assert_eq!(rlp_encode_u64(0), vec![0x80]);
        assert_eq!(rlp_encode_u64(1), vec![0x01]);
        assert_eq!(rlp_encode_u64(127), vec![0x7f]);
        assert_eq!(rlp_encode_u64(128), vec![0x81, 0x80]);
    }

    #[test]
    fn test_rlp_encode_bytes() {
        assert_eq!(rlp_encode_bytes(&[]), vec![0x80]);
        assert_eq!(rlp_encode_bytes(&[0x42]), vec![0x42]);
        assert_eq!(rlp_encode_bytes(&[0x80]), vec![0x81, 0x80]);
    }

    #[test]
    fn test_rlp_encode_list() {
        // Empty list
        assert_eq!(rlp_encode_list(&[]), vec![0xc0]);
    }

    #[test]
    fn test_parse_address() {
        let addr = parse_address("0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045").unwrap();
        assert_eq!(addr.len(), 20);

        // Without 0x prefix
        let addr2 = parse_address("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045").unwrap();
        assert_eq!(addr, addr2);
    }

    #[test]
    fn test_parse_address_invalid() {
        assert!(parse_address("0xshort").is_err());
        assert!(parse_address("0x").is_err());
    }

    #[test]
    fn test_build_and_sign_eth_tx() {
        // Generate a key
        use k256::ecdsa::SigningKey;
        let signing_key = SigningKey::random(&mut rand::rngs::OsRng);
        let key_bytes = signing_key.to_bytes().to_vec();

        let params = EthTxParams {
            chain_id: 1,
            nonce: 0,
            to: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045".to_string(),
            value: "1000000000000000000".to_string(), // 1 ETH in wei
            max_fee_per_gas: 30_000_000_000,
            max_priority_fee_per_gas: 1_000_000_000,
            gas_limit: 21_000,
            data: None,
        };

        let result = build_and_sign_eip1559(&key_bytes, &params).unwrap();

        // Signed tx should start with 0x02 (EIP-1559 type)
        assert!(result.signed_tx.starts_with("0x02"));
        // Tx hash should be 0x + 64 hex chars
        assert!(result.tx_hash.starts_with("0x"));
        assert_eq!(result.tx_hash.len(), 66);
        // From should be valid address
        assert!(result.from.starts_with("0x"));
        assert_eq!(result.from.len(), 42);
        assert_eq!(result.chain_id, 1);
    }

    #[test]
    fn test_build_eth_tx_with_data() {
        use k256::ecdsa::SigningKey;
        let signing_key = SigningKey::random(&mut rand::rngs::OsRng);
        let key_bytes = signing_key.to_bytes().to_vec();

        let params = EthTxParams {
            chain_id: 11155111, // Sepolia
            nonce: 5,
            to: "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045".to_string(),
            value: "0".to_string(),
            max_fee_per_gas: 50_000_000_000,
            max_priority_fee_per_gas: 2_000_000_000,
            gas_limit: 100_000,
            data: Some("a9059cbb".to_string()), // ERC-20 transfer selector
        };

        let result = build_and_sign_eip1559(&key_bytes, &params).unwrap();
        assert!(result.signed_tx.starts_with("0x02"));
    }

    #[test]
    fn test_keccak256_known_vector() {
        let hash = keccak256(b"");
        assert_eq!(
            hex::encode(hash),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn test_strip_leading_zeros() {
        assert_eq!(strip_leading_zeros(&[0, 0, 1, 2]), &[1, 2]);
        assert_eq!(strip_leading_zeros(&[1, 2, 3]), &[1, 2, 3]);
        assert_eq!(strip_leading_zeros(&[0, 0, 0]), &[0]);
    }
}
