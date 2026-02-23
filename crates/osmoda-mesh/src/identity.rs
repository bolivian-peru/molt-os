use std::path::Path;

use anyhow::{Context, Result};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use kem::{Decapsulate, Encapsulate};
use ml_kem::{EncodedSizeUser, KemCore, MlKem768};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

/// Public identity of a mesh peer. Signed by Ed25519 for authenticity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshPublicIdentity {
    pub instance_id: String,
    pub ed25519_pubkey: String,
    pub noise_static_pubkey: String,
    pub mlkem_encap_key: String,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// Complete mesh identity including private keys. Zeroized on drop.
pub struct MeshIdentity {
    pub ed25519_signing_key: SigningKey,
    pub noise_static_keypair: snow::Keypair,
    pub mlkem_dk_bytes: Vec<u8>,
    pub mlkem_ek_bytes: Vec<u8>,
    pub public_identity: MeshPublicIdentity,
}

impl Drop for MeshIdentity {
    fn drop(&mut self) {
        self.mlkem_dk_bytes.zeroize();
        self.noise_static_keypair.private.zeroize();
    }
}

impl MeshIdentity {
    /// Load an existing identity from disk, or generate a new one on first boot.
    pub fn load_or_create(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;

        let ed25519_path = data_dir.join("ed25519.key");
        let noise_priv_path = data_dir.join("noise_static.key");
        let noise_pub_path = data_dir.join("noise_static.pub");
        let mlkem_dk_path = data_dir.join("mlkem.dk");

        if ed25519_path.exists()
            && noise_priv_path.exists()
            && noise_pub_path.exists()
            && mlkem_dk_path.exists()
        {
            Self::load_from_disk(data_dir)
        } else {
            let identity = Self::generate()?;
            identity.save_to_disk(data_dir)?;

            let identity_json = serde_json::to_string_pretty(&identity.public_identity)?;
            std::fs::write(data_dir.join("identity.json"), identity_json)?;

            tracing::info!(
                instance_id = %identity.public_identity.instance_id,
                "generated new mesh identity"
            );
            Ok(identity)
        }
    }

    /// Generate a brand new mesh identity.
    fn generate() -> Result<Self> {
        // 1. Ed25519 signing key
        let mut ed25519_secret = [0u8; 32];
        OsRng.fill_bytes(&mut ed25519_secret);
        let ed25519_signing_key = SigningKey::from_bytes(&ed25519_secret);
        ed25519_secret.zeroize();
        let ed25519_verifying_key = ed25519_signing_key.verifying_key();

        // 2. Noise X25519 static keypair (snow generates both private + public)
        let noise_static_keypair =
            snow::Builder::new("Noise_XX_25519_ChaChaPoly_BLAKE2s".parse()?).generate_keypair()?;

        // 3. ML-KEM-768 keypair
        let (dk, ek) = MlKem768::generate(&mut OsRng);
        let dk_bytes = dk.as_bytes().to_vec();
        let ek_bytes = ek.as_bytes().to_vec();

        // 4. Derive instance_id = hex(SHA-256(noise_static_pubkey))[..32]
        let mut hasher = Sha256::new();
        hasher.update(&noise_static_keypair.public);
        let hash = hasher.finalize();
        let instance_id = hex::encode(&hash[..16]); // 16 bytes = 32 hex chars

        // 5. Build public identity
        let mut public_identity = MeshPublicIdentity {
            instance_id,
            ed25519_pubkey: hex::encode(ed25519_verifying_key.as_bytes()),
            noise_static_pubkey: hex::encode(&noise_static_keypair.public),
            mlkem_encap_key: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &ek_bytes,
            ),
            capabilities: vec!["mesh.v1".to_string()],
            signature: None,
        };

        // 6. Sign the identity
        let sig = Self::sign_identity(&ed25519_signing_key, &public_identity)?;
        public_identity.signature = Some(hex::encode(sig));

        Ok(Self {
            ed25519_signing_key,
            noise_static_keypair,
            mlkem_dk_bytes: dk_bytes,
            mlkem_ek_bytes: ek_bytes,
            public_identity,
        })
    }

    /// Sign the canonical JSON of the public identity (excluding the signature field).
    fn sign_identity(signing_key: &SigningKey, identity: &MeshPublicIdentity) -> Result<Vec<u8>> {
        let canonical = Self::canonical_json(identity)?;
        let signature = signing_key.sign(canonical.as_bytes());
        Ok(signature.to_bytes().to_vec())
    }

    /// Produce canonical JSON for signing (signature field excluded).
    fn canonical_json(identity: &MeshPublicIdentity) -> Result<String> {
        let signable = serde_json::json!({
            "instance_id": identity.instance_id,
            "ed25519_pubkey": identity.ed25519_pubkey,
            "noise_static_pubkey": identity.noise_static_pubkey,
            "mlkem_encap_key": identity.mlkem_encap_key,
            "capabilities": identity.capabilities,
        });
        Ok(serde_json::to_string(&signable)?)
    }

    /// Verify the signature on a public identity.
    pub fn verify_identity(identity: &MeshPublicIdentity) -> Result<bool> {
        let sig_hex = identity
            .signature
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("identity has no signature"))?;

        let sig_bytes = hex::decode(sig_hex)?;
        if sig_bytes.len() != 64 {
            anyhow::bail!("invalid signature length: {}", sig_bytes.len());
        }
        let signature = ed25519_dalek::Signature::from_slice(&sig_bytes)?;

        let pubkey_bytes = hex::decode(&identity.ed25519_pubkey)?;
        if pubkey_bytes.len() != 32 {
            anyhow::bail!("invalid ed25519 pubkey length: {}", pubkey_bytes.len());
        }
        let mut pk_arr = [0u8; 32];
        pk_arr.copy_from_slice(&pubkey_bytes);
        let verifying_key = VerifyingKey::from_bytes(&pk_arr)?;

        let canonical = Self::canonical_json(identity)?;
        Ok(verifying_key
            .verify_strict(canonical.as_bytes(), &signature)
            .is_ok())
    }

    /// Save private keys to disk with 0o600 permissions.
    fn save_to_disk(&self, data_dir: &Path) -> Result<()> {
        let write_secret = |path: &Path, data: &[u8]| -> Result<()> {
            std::fs::write(path, data)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            }
            Ok(())
        };

        write_secret(
            &data_dir.join("ed25519.key"),
            &self.ed25519_signing_key.to_bytes(),
        )?;
        write_secret(
            &data_dir.join("noise_static.key"),
            &self.noise_static_keypair.private,
        )?;
        // Save public key alongside private so we can reconstruct the Keypair on load
        std::fs::write(
            data_dir.join("noise_static.pub"),
            &self.noise_static_keypair.public,
        )?;
        write_secret(&data_dir.join("mlkem.dk"), &self.mlkem_dk_bytes)?;

        Ok(())
    }

    /// Load identity from disk.
    fn load_from_disk(data_dir: &Path) -> Result<Self> {
        // Ed25519
        let ed25519_bytes =
            std::fs::read(data_dir.join("ed25519.key")).context("failed to read ed25519 key")?;
        if ed25519_bytes.len() != 32 {
            anyhow::bail!("ed25519 key has invalid length: {}", ed25519_bytes.len());
        }
        let mut ed25519_arr = [0u8; 32];
        ed25519_arr.copy_from_slice(&ed25519_bytes);
        let ed25519_signing_key = SigningKey::from_bytes(&ed25519_arr);
        ed25519_arr.zeroize();
        let ed25519_verifying_key = ed25519_signing_key.verifying_key();

        // Noise X25519 — load both private and public
        let noise_private =
            std::fs::read(data_dir.join("noise_static.key")).context("failed to read noise key")?;
        let noise_public = std::fs::read(data_dir.join("noise_static.pub"))
            .context("failed to read noise public key")?;
        if noise_private.len() != 32 {
            anyhow::bail!(
                "noise private key has invalid length: {}",
                noise_private.len()
            );
        }
        if noise_public.len() != 32 {
            anyhow::bail!(
                "noise public key has invalid length: {}",
                noise_public.len()
            );
        }
        let noise_static_keypair = snow::Keypair {
            private: noise_private,
            public: noise_public,
        };

        // ML-KEM
        let mlkem_dk_bytes =
            std::fs::read(data_dir.join("mlkem.dk")).context("failed to read ML-KEM dk")?;
        // Derive encapsulation key from decapsulation key
        let dk = <MlKem768 as KemCore>::DecapsulationKey::from_bytes(
            mlkem_dk_bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid ML-KEM DK length"))?,
        );
        let mlkem_ek_bytes = dk.encapsulation_key().as_bytes().to_vec();

        // Derive instance_id
        let mut hasher = Sha256::new();
        hasher.update(&noise_static_keypair.public);
        let hash = hasher.finalize();
        let instance_id = hex::encode(&hash[..16]);

        let mut public_identity = MeshPublicIdentity {
            instance_id,
            ed25519_pubkey: hex::encode(ed25519_verifying_key.as_bytes()),
            noise_static_pubkey: hex::encode(&noise_static_keypair.public),
            mlkem_encap_key: base64::Engine::encode(
                &base64::engine::general_purpose::STANDARD,
                &mlkem_ek_bytes,
            ),
            capabilities: vec!["mesh.v1".to_string()],
            signature: None,
        };

        let sig = Self::sign_identity(&ed25519_signing_key, &public_identity)?;
        public_identity.signature = Some(hex::encode(sig));

        Ok(Self {
            ed25519_signing_key,
            noise_static_keypair,
            mlkem_dk_bytes,
            mlkem_ek_bytes,
            public_identity,
        })
    }

    /// Encapsulate a shared secret to a peer's ML-KEM encapsulation key.
    /// Returns (ciphertext_bytes, shared_secret_bytes).
    pub fn mlkem_encapsulate(peer_ek_base64: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        use base64::Engine;
        let ek_bytes = base64::engine::general_purpose::STANDARD.decode(peer_ek_base64)?;
        let ek = <MlKem768 as KemCore>::EncapsulationKey::from_bytes(
            ek_bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid ML-KEM EK length"))?,
        );
        let (ct, ss) = ek
            .encapsulate(&mut OsRng)
            .map_err(|_| anyhow::anyhow!("ML-KEM encapsulation failed"))?;
        Ok((ct.to_vec(), ss.to_vec()))
    }

    /// Decapsulate a shared secret from a ciphertext using our decapsulation key.
    pub fn mlkem_decapsulate(&self, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let dk = <MlKem768 as KemCore>::DecapsulationKey::from_bytes(
            self.mlkem_dk_bytes
                .as_slice()
                .try_into()
                .map_err(|_| anyhow::anyhow!("invalid ML-KEM DK length"))?,
        );
        let ct: &ml_kem::Ciphertext<MlKem768> = ciphertext
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid ML-KEM ciphertext length"))?;
        let ss = dk
            .decapsulate(ct)
            .map_err(|_| anyhow::anyhow!("ML-KEM decapsulation failed"))?;
        Ok(ss.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_and_verify_identity() {
        let identity = MeshIdentity::generate().unwrap();
        assert!(!identity.public_identity.instance_id.is_empty());
        assert_eq!(identity.public_identity.instance_id.len(), 32);
        assert!(identity.public_identity.signature.is_some());

        let valid = MeshIdentity::verify_identity(&identity.public_identity).unwrap();
        assert!(valid, "identity signature should be valid");
    }

    #[test]
    fn test_identity_persist_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let id1 = MeshIdentity::load_or_create(dir.path()).unwrap();
        let id2 = MeshIdentity::load_or_create(dir.path()).unwrap();

        assert_eq!(
            id1.public_identity.instance_id,
            id2.public_identity.instance_id
        );
        assert_eq!(
            id1.public_identity.ed25519_pubkey,
            id2.public_identity.ed25519_pubkey
        );
    }

    #[test]
    fn test_instance_id_derivation() {
        let identity = MeshIdentity::generate().unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&identity.noise_static_keypair.public);
        let hash = hasher.finalize();
        let expected = hex::encode(&hash[..16]);
        assert_eq!(identity.public_identity.instance_id, expected);
    }

    #[test]
    fn test_mlkem_encapsulate_decapsulate_roundtrip() {
        let identity = MeshIdentity::generate().unwrap();
        let (ciphertext, shared_secret_enc) =
            MeshIdentity::mlkem_encapsulate(&identity.public_identity.mlkem_encap_key).unwrap();
        let shared_secret_dec = identity.mlkem_decapsulate(&ciphertext).unwrap();
        assert_eq!(shared_secret_enc, shared_secret_dec);
    }

    #[test]
    fn test_tampered_signature_fails() {
        let identity = MeshIdentity::generate().unwrap();
        let mut tampered = identity.public_identity.clone();
        tampered.instance_id = "tampered_id_value_padded_to_32c".to_string();
        let valid = MeshIdentity::verify_identity(&tampered).unwrap();
        assert!(!valid, "tampered identity should fail verification");
    }
}
