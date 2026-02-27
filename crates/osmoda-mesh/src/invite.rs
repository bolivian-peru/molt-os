use anyhow::Result;
use base64::Engine;
use serde::{Deserialize, Serialize};

/// Default invite TTL: 1 hour.
const DEFAULT_INVITE_TTL_SECS: u64 = 3600;

/// Out-of-band invite payload. Encoded as base64url for easy copy-paste.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvitePayload {
    pub endpoint: String,
    pub noise_static_pubkey: String,
    pub mlkem_encap_key: String,
    pub instance_id: String,
    pub expires_at: String,
}

/// Maximum invite TTL: 1 hour. Prevents long-lived invite codes.
const MAX_INVITE_TTL_SECS: u64 = 3600;

impl InvitePayload {
    /// Create a new invite from our identity and listen address.
    pub fn new(
        endpoint: &str,
        noise_static_pubkey: &str,
        mlkem_encap_key: &str,
        instance_id: &str,
        ttl_secs: Option<u64>,
    ) -> Self {
        // Clamp TTL to MAX_INVITE_TTL_SECS (1 hour)
        let ttl = ttl_secs.unwrap_or(DEFAULT_INVITE_TTL_SECS).min(MAX_INVITE_TTL_SECS);
        let expires_at = chrono::Utc::now()
            + chrono::Duration::seconds(ttl as i64);

        Self {
            endpoint: endpoint.to_string(),
            noise_static_pubkey: noise_static_pubkey.to_string(),
            mlkem_encap_key: mlkem_encap_key.to_string(),
            instance_id: instance_id.to_string(),
            expires_at: expires_at.to_rfc3339(),
        }
    }

    /// Encode the invite as a base64url string (copy-pasteable).
    pub fn encode(&self) -> Result<String> {
        let json = serde_json::to_string(self)?;
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json.as_bytes()))
    }

    /// Decode an invite from a base64url string. Validates expiry.
    pub fn decode(code: &str) -> Result<Self> {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(code)?;
        let json = String::from_utf8(bytes)?;
        let payload: Self = serde_json::from_str(&json)?;

        // Validate expiry
        let expires = chrono::DateTime::parse_from_rfc3339(&payload.expires_at)
            .map_err(|e| anyhow::anyhow!("invalid expires_at: {e}"))?;
        if chrono::Utc::now() > expires {
            anyhow::bail!("invite has expired (expired at {})", payload.expires_at);
        }

        Ok(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_roundtrip() {
        let invite = InvitePayload::new(
            "192.168.1.1:18800",
            "aabbccdd",
            "base64_ek_data",
            "instance123",
            Some(3600),
        );

        let code = invite.encode().unwrap();
        assert!(!code.is_empty());

        let decoded = InvitePayload::decode(&code).unwrap();
        assert_eq!(decoded.endpoint, "192.168.1.1:18800");
        assert_eq!(decoded.noise_static_pubkey, "aabbccdd");
        assert_eq!(decoded.mlkem_encap_key, "base64_ek_data");
        assert_eq!(decoded.instance_id, "instance123");
    }

    #[test]
    fn test_expired_invite_rejected() {
        let payload = InvitePayload {
            endpoint: "127.0.0.1:18800".to_string(),
            noise_static_pubkey: "abc".to_string(),
            mlkem_encap_key: "def".to_string(),
            instance_id: "id1".to_string(),
            expires_at: "2020-01-01T00:00:00+00:00".to_string(), // in the past
        };

        let code = payload.encode().unwrap();
        let result = InvitePayload::decode(&code);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().to_string().contains("expired"),
            "should reject expired invite"
        );
    }

    #[test]
    fn test_invalid_base64_rejected() {
        let result = InvitePayload::decode("not-valid-base64!!!");
        assert!(result.is_err());
    }
}
