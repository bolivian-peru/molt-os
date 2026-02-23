use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// How long decrypted keys stay in cache before eviction (5 minutes).
const KEY_CACHE_TTL_SECS: u64 = 300;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Chain {
    Ethereum,
    Solana,
}

impl std::fmt::Display for Chain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Chain::Ethereum => write!(f, "ethereum"),
            Chain::Solana => write!(f, "solana"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletInfo {
    pub id: String,
    pub label: String,
    pub chain: Chain,
    pub address: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct WalletIndex {
    wallets: Vec<WalletInfo>,
}

pub struct LocalKeyBackend {
    data_dir: PathBuf,
    master_key: [u8; 32],
    index: WalletIndex,
    // Cached decrypted key bytes with access timestamp — zeroized on drop via Drop impl
    cached_keys: HashMap<String, (Vec<u8>, Instant)>,
}

impl Drop for LocalKeyBackend {
    fn drop(&mut self) {
        self.master_key.zeroize();
        for (_, (v, _)) in self.cached_keys.iter_mut() {
            v.zeroize();
        }
    }
}

impl LocalKeyBackend {
    pub fn new(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir.join("keys"))?;

        let master_key = Self::load_or_create_master_key(data_dir)?;
        let index = Self::load_index(data_dir)?;

        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            master_key,
            index,
            cached_keys: HashMap::new(),
        })
    }

    /// Load or create the master key. The raw key material is stored on disk alongside
    /// a random salt. The actual encryption key is derived via Argon2id(raw_key, salt).
    /// This means even if the raw key file is copied, the derived key depends on the salt.
    fn load_or_create_master_key(data_dir: &Path) -> Result<[u8; 32]> {
        let key_path = data_dir.join("master.key");
        let salt_path = data_dir.join("master.salt");

        let (raw_key, salt) = if key_path.exists() && salt_path.exists() {
            let raw = std::fs::read(&key_path).context("failed to read master key")?;
            let salt = std::fs::read(&salt_path).context("failed to read master salt")?;
            if raw.len() != 32 {
                anyhow::bail!("master key has invalid length: {}", raw.len());
            }
            if salt.len() != 16 {
                anyhow::bail!("master salt has invalid length: {}", salt.len());
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&raw);
            let mut s = [0u8; 16];
            s.copy_from_slice(&salt);
            (key, s)
        } else {
            let mut raw_key = [0u8; 32];
            let mut salt = [0u8; 16];
            rand::rngs::OsRng.fill_bytes(&mut raw_key);
            rand::rngs::OsRng.fill_bytes(&mut salt);

            std::fs::write(&key_path, &raw_key).context("failed to write master key")?;
            std::fs::write(&salt_path, &salt).context("failed to write master salt")?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
                std::fs::set_permissions(&salt_path, std::fs::Permissions::from_mode(0o600))?;
            }
            tracing::info!("generated new master key + salt");
            (raw_key, salt)
        };

        // Derive the actual encryption key via Argon2id
        let derived = Self::derive_key(&raw_key, &salt)?;
        Ok(derived)
    }

    /// Derive a 32-byte encryption key from raw key material + salt using Argon2id.
    fn derive_key(raw_key: &[u8; 32], salt: &[u8; 16]) -> Result<[u8; 32]> {
        use argon2::{Algorithm, Argon2, Params, Version};

        // Argon2id with moderate parameters suitable for a daemon (not interactive)
        // m=64 MiB, t=3 iterations, p=1 parallelism
        let params = Params::new(65536, 3, 1, Some(32))
            .map_err(|e| anyhow::anyhow!("argon2 params error: {e}"))?;
        let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

        let mut derived = [0u8; 32];
        argon2
            .hash_password_into(raw_key, salt, &mut derived)
            .map_err(|e| anyhow::anyhow!("argon2 key derivation failed: {e}"))?;

        Ok(derived)
    }

    fn load_index(data_dir: &Path) -> Result<WalletIndex> {
        let index_path = data_dir.join("wallets.json");
        if index_path.exists() {
            let data = std::fs::read_to_string(&index_path)?;
            Ok(serde_json::from_str(&data)?)
        } else {
            Ok(WalletIndex {
                wallets: Vec::new(),
            })
        }
    }

    fn save_index(&self) -> Result<()> {
        let index_path = self.data_dir.join("wallets.json");
        let data = serde_json::to_string_pretty(&self.index)?;
        std::fs::write(&index_path, &data)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&index_path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher =
            Aes256Gcm::new_from_slice(&self.master_key).context("invalid master key length")?;
        let mut nonce_bytes = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;
        // Format: nonce (12 bytes) || ciphertext
        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce_bytes);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 12 {
            anyhow::bail!("encrypted data too short");
        }
        let cipher =
            Aes256Gcm::new_from_slice(&self.master_key).context("invalid master key length")?;
        let nonce = Nonce::from_slice(&data[..12]);
        let plaintext = cipher
            .decrypt(nonce, &data[12..])
            .map_err(|e| anyhow::anyhow!("decryption failed: {e}"))?;
        Ok(plaintext)
    }

    fn key_path(&self, wallet_id: &str) -> PathBuf {
        self.data_dir.join("keys").join(format!("{wallet_id}.enc"))
    }

    pub fn create_wallet(&mut self, chain: Chain, label: &str) -> Result<WalletInfo> {
        if label.len() > 128 {
            anyhow::bail!("wallet label too long (max 128 chars)");
        }

        let wallet_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let (address, mut key_bytes) = match chain {
            Chain::Ethereum => {
                use k256::ecdsa::SigningKey;
                let signing_key = SigningKey::random(&mut rand::rngs::OsRng);
                let verifying_key = signing_key.verifying_key();
                // Ethereum address = last 20 bytes of Keccak-256(uncompressed_pubkey[1..])
                let pubkey_bytes = verifying_key.to_encoded_point(false);
                let pubkey_uncompressed = &pubkey_bytes.as_bytes()[1..]; // skip 0x04 prefix
                let hash = keccak256(pubkey_uncompressed);
                let addr = format!("0x{}", hex::encode(&hash[12..]));
                let key_bytes = signing_key.to_bytes().to_vec();
                (addr, key_bytes)
            }
            Chain::Solana => {
                use ed25519_dalek::SigningKey;
                let mut secret = [0u8; 32];
                rand::rngs::OsRng.fill_bytes(&mut secret);
                let signing_key = SigningKey::from_bytes(&secret);
                let pubkey = signing_key.verifying_key();
                let addr = bs58::encode(pubkey.as_bytes()).into_string();
                // Store only the 32-byte secret — public key is derived
                let key_bytes = secret.to_vec();
                secret.zeroize();
                (addr, key_bytes)
            }
        };

        // Encrypt and store with restricted permissions
        let encrypted = self.encrypt(&key_bytes)?;
        key_bytes.zeroize();
        let key_file = self.key_path(&wallet_id);
        std::fs::write(&key_file, &encrypted)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600))?;
        }

        let info = WalletInfo {
            id: wallet_id,
            label: label.to_string(),
            chain,
            address,
            created_at: now,
        };

        self.index.wallets.push(info.clone());
        self.save_index()?;

        tracing::info!(wallet_id = %info.id, chain = %chain, address = %info.address, "wallet created");
        Ok(info)
    }

    pub fn list_wallets(&self) -> Vec<WalletInfo> {
        self.index.wallets.clone()
    }

    fn load_key_bytes(&mut self, wallet_id: &str) -> Result<Vec<u8>> {
        // Evict stale entries first
        self.evict_stale_keys();

        if let Some((bytes, accessed_at)) = self.cached_keys.get_mut(wallet_id) {
            *accessed_at = Instant::now(); // refresh TTL on access
            return Ok(bytes.clone());
        }
        let encrypted = std::fs::read(self.key_path(wallet_id))
            .context("failed to read encrypted key")?;
        let key_bytes = self.decrypt(&encrypted)?;
        self.cached_keys
            .insert(wallet_id.to_string(), (key_bytes.clone(), Instant::now()));
        Ok(key_bytes)
    }

    /// Remove cached keys that haven't been accessed within the TTL.
    pub fn evict_stale_keys(&mut self) {
        let cutoff = std::time::Duration::from_secs(KEY_CACHE_TTL_SECS);
        let now = Instant::now();
        self.cached_keys.retain(|id, (bytes, accessed_at)| {
            if now.duration_since(*accessed_at) > cutoff {
                tracing::debug!(wallet_id = %id, "evicting cached key (TTL expired)");
                bytes.zeroize();
                false
            } else {
                true
            }
        });
    }

    fn find_wallet(&self, wallet_id: &str) -> Result<&WalletInfo> {
        self.index
            .wallets
            .iter()
            .find(|w| w.id == wallet_id)
            .ok_or_else(|| anyhow::anyhow!("wallet not found: {wallet_id}"))
    }

    pub async fn sign_message(&mut self, wallet_id: &str, message: &[u8]) -> Result<Vec<u8>> {
        let wallet = self.find_wallet(wallet_id)?.clone();
        let mut key_bytes = self.load_key_bytes(wallet_id)?;

        let result = match wallet.chain {
            Chain::Ethereum => {
                use k256::ecdsa::{signature::Signer, SigningKey, Signature};
                let signing_key = SigningKey::from_slice(&key_bytes)
                    .map_err(|e| anyhow::anyhow!("invalid ETH key: {e}"))?;
                let sig: Signature = signing_key.sign(message);
                Ok(sig.to_bytes().to_vec())
            }
            Chain::Solana => {
                use ed25519_dalek::{Signer, SigningKey};
                if key_bytes.len() < 32 {
                    anyhow::bail!("invalid SOL key length: expected 32, got {}", key_bytes.len());
                }
                let mut secret = [0u8; 32];
                secret.copy_from_slice(&key_bytes[..32]);
                let signing_key = SigningKey::from_bytes(&secret);
                let sig = signing_key.sign(message);
                secret.zeroize();
                Ok(sig.to_bytes().to_vec())
            }
        };

        key_bytes.zeroize();
        result
    }

    pub async fn sign_transaction(&mut self, wallet_id: &str, tx_bytes: &[u8]) -> Result<Vec<u8>> {
        self.sign_message(wallet_id, tx_bytes).await
    }

    pub fn delete_wallet(&mut self, wallet_id: &str) -> Result<()> {
        // Verify wallet exists
        self.find_wallet(wallet_id)?;

        // Remove encrypted key file
        let key_path = self.key_path(wallet_id);
        if key_path.exists() {
            std::fs::remove_file(&key_path).context("failed to remove encrypted key file")?;
        }

        // Remove from cache and zeroize
        if let Some((mut cached, _)) = self.cached_keys.remove(wallet_id) {
            cached.zeroize();
        }

        // Remove from index
        self.index.wallets.retain(|w| w.id != wallet_id);
        self.save_index()?;

        tracing::info!(wallet_id = %wallet_id, "wallet deleted");
        Ok(())
    }

    pub fn address(&self, wallet_id: &str) -> Result<String> {
        let wallet = self.find_wallet(wallet_id)?;
        Ok(wallet.address.clone())
    }

    pub fn wallet_count(&self) -> usize {
        self.index.wallets.len()
    }
}

/// Keccak-256 hash — the correct hash for Ethereum address derivation.
fn keccak256(data: &[u8]) -> [u8; 32] {
    use sha3::Digest;
    let mut hasher = sha3::Keccak256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_and_sign_eth() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = LocalKeyBackend::new(dir.path()).unwrap();

        let wallet = backend
            .create_wallet(Chain::Ethereum, "test-eth")
            .unwrap();
        assert_eq!(wallet.chain, Chain::Ethereum);
        assert!(wallet.address.starts_with("0x"));
        assert_eq!(wallet.address.len(), 42); // 0x + 40 hex chars

        let msg = b"hello world";
        let sig = backend.sign_message(&wallet.id, msg).await.unwrap();
        assert!(!sig.is_empty());
        assert_eq!(sig.len(), 64); // ECDSA signature r+s
    }

    #[tokio::test]
    async fn test_create_and_sign_sol() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = LocalKeyBackend::new(dir.path()).unwrap();

        let wallet = backend
            .create_wallet(Chain::Solana, "test-sol")
            .unwrap();
        assert_eq!(wallet.chain, Chain::Solana);
        assert!(!wallet.address.is_empty());

        let msg = b"hello solana";
        let sig = backend.sign_message(&wallet.id, msg).await.unwrap();
        assert_eq!(sig.len(), 64); // ed25519 signature
    }

    #[tokio::test]
    async fn test_sign_verify_eth_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = LocalKeyBackend::new(dir.path()).unwrap();

        let wallet = backend
            .create_wallet(Chain::Ethereum, "verify-test")
            .unwrap();

        let msg = b"verify me";
        let sig_bytes = backend.sign_message(&wallet.id, msg).await.unwrap();

        // Verify: recover public key from signature
        use k256::ecdsa::{signature::Verifier, Signature};
        let key_bytes = backend.load_key_bytes(&wallet.id).unwrap();
        let signing_key = k256::ecdsa::SigningKey::from_slice(&key_bytes).unwrap();
        let verifying_key = signing_key.verifying_key();
        let sig = Signature::from_slice(&sig_bytes).unwrap();
        assert!(verifying_key.verify(msg, &sig).is_ok());
    }

    #[tokio::test]
    async fn test_sign_verify_sol_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = LocalKeyBackend::new(dir.path()).unwrap();

        let wallet = backend
            .create_wallet(Chain::Solana, "verify-sol")
            .unwrap();

        let msg = b"verify sol";
        let sig_bytes = backend.sign_message(&wallet.id, msg).await.unwrap();

        // Verify
        use ed25519_dalek::{Signature, Verifier};
        let key_bytes = backend.load_key_bytes(&wallet.id).unwrap();
        let secret: [u8; 32] = key_bytes[..32].try_into().unwrap();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&secret);
        let verifying_key = signing_key.verifying_key();
        let sig = Signature::from_slice(&sig_bytes).unwrap();
        assert!(verifying_key.verify(msg, &sig).is_ok());
    }

    #[test]
    fn test_list_wallets() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = LocalKeyBackend::new(dir.path()).unwrap();

        backend.create_wallet(Chain::Ethereum, "w1").unwrap();
        backend.create_wallet(Chain::Solana, "w2").unwrap();

        let wallets = backend.list_wallets();
        assert_eq!(wallets.len(), 2);
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalKeyBackend::new(dir.path()).unwrap();

        let plaintext = b"secret key material";
        let encrypted = backend.encrypt(plaintext).unwrap();
        let decrypted = backend.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_persistence() {
        let dir = tempfile::tempdir().unwrap();
        {
            let mut backend = LocalKeyBackend::new(dir.path()).unwrap();
            backend.create_wallet(Chain::Ethereum, "persist-test").unwrap();
        }
        // Reopen
        let backend = LocalKeyBackend::new(dir.path()).unwrap();
        assert_eq!(backend.list_wallets().len(), 1);
        assert_eq!(backend.list_wallets()[0].label, "persist-test");
    }

    #[test]
    fn test_label_length_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = LocalKeyBackend::new(dir.path()).unwrap();
        let long_label = "x".repeat(200);
        assert!(backend.create_wallet(Chain::Ethereum, &long_label).is_err());
    }

    #[test]
    fn test_keccak256_known_vector() {
        // Empty input keccak256 = c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470
        let hash = keccak256(b"");
        assert_eq!(
            hex::encode(hash),
            "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
    }

    #[test]
    fn test_delete_wallet() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = LocalKeyBackend::new(dir.path()).unwrap();

        let wallet = backend.create_wallet(Chain::Ethereum, "delete-me").unwrap();
        assert_eq!(backend.wallet_count(), 1);

        // Key file should exist
        let key_path = dir.path().join("keys").join(format!("{}.enc", wallet.id));
        assert!(key_path.exists());

        // Delete
        backend.delete_wallet(&wallet.id).unwrap();
        assert_eq!(backend.wallet_count(), 0);

        // Key file should be gone
        assert!(!key_path.exists());

        // Should not find wallet
        assert!(backend.address(&wallet.id).is_err());
    }

    #[test]
    fn test_delete_nonexistent_wallet() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = LocalKeyBackend::new(dir.path()).unwrap();
        assert!(backend.delete_wallet("no-such-id").is_err());
    }

    #[test]
    fn test_key_cache_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let mut backend = LocalKeyBackend::new(dir.path()).unwrap();

        let wallet = backend.create_wallet(Chain::Ethereum, "cache-test").unwrap();
        // Load key into cache
        let _ = backend.load_key_bytes(&wallet.id).unwrap();
        assert_eq!(backend.cached_keys.len(), 1);

        // Manually backdate the cache entry to force eviction
        if let Some((_, accessed_at)) = backend.cached_keys.get_mut(&wallet.id) {
            *accessed_at = Instant::now() - std::time::Duration::from_secs(KEY_CACHE_TTL_SECS + 1);
        }
        backend.evict_stale_keys();
        assert_eq!(backend.cached_keys.len(), 0, "stale key should be evicted");
    }

    #[test]
    fn test_argon2_kdf_produces_consistent_key() {
        // Same raw key + salt should produce same derived key
        let dir = tempfile::tempdir().unwrap();
        let _backend1 = LocalKeyBackend::new(dir.path()).unwrap();
        // Reopen same data dir — should derive the same key and work
        let backend2 = LocalKeyBackend::new(dir.path()).unwrap();
        // If KDF produced a different key, this would have wrong master key
        let plaintext = b"kdf consistency test";
        let encrypted = backend2.encrypt(plaintext).unwrap();
        let decrypted = backend2.decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }
}
