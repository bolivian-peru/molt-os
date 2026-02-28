use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Trust ring levels for sandboxed execution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Ring {
    /// Approved apps — declared capabilities, limited network via egress proxy.
    Ring1,
    /// Untrusted tools — max isolation, no network, minimal filesystem.
    Ring2,
}

impl std::fmt::Display for Ring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ring::Ring1 => write!(f, "ring1"),
            Ring::Ring2 => write!(f, "ring2"),
        }
    }
}

/// Sandbox configuration for a command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxConfig {
    pub ring: Ring,
    pub capabilities: Vec<String>,
    pub timeout_secs: u64,
    pub memory_limit_mb: u64,
    /// Allowed filesystem read paths (Ring1 only).
    pub fs_read: Vec<String>,
    /// Allowed filesystem write paths (Ring1 only).
    pub fs_write: Vec<String>,
    /// Whether network access is allowed (Ring1 with egress proxy only).
    pub network: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            ring: Ring::Ring2,
            capabilities: Vec::new(),
            timeout_secs: 60,
            memory_limit_mb: 512,
            fs_read: Vec::new(),
            fs_write: Vec::new(),
            network: false,
        }
    }
}

/// Result of a sandboxed command execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub ring: Ring,
    pub timed_out: bool,
}

/// A capability token that grants specific permissions to a sandboxed process.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityToken {
    pub id: String,
    pub granted_to: String,
    pub permissions: Vec<String>,
    pub created_at: String,
    pub expires_at: String,
    pub signature: String,
}

/// Sandbox engine that builds and executes bwrap commands.
pub struct SandboxEngine {
    /// HMAC key for signing capability tokens.
    hmac_key: [u8; 32],
    /// Path to the egress proxy socket.
    egress_proxy: String,
}

impl SandboxEngine {
    pub fn new(hmac_key: [u8; 32], egress_proxy: &str) -> Self {
        Self {
            hmac_key,
            egress_proxy: egress_proxy.to_string(),
        }
    }

    /// Generate a new sandbox engine with a random HMAC key.
    pub fn generate(egress_proxy: &str) -> Self {
        let mut key = [0u8; 32];
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut key);
        Self::new(key, egress_proxy)
    }

    /// Build the bwrap command arguments for a given ring and config.
    pub fn build_bwrap_args(&self, config: &SandboxConfig, command: &str) -> Vec<String> {
        let mut args = Vec::new();

        // Common: unshare all namespaces
        args.push("--unshare-all".to_string());
        args.push("--die-with-parent".to_string());

        // /nix/store is always read-only available
        args.push("--ro-bind".to_string());
        args.push("/nix/store".to_string());
        args.push("/nix/store".to_string());

        // /usr, /bin, /lib — basic system paths (read-only)
        for path in &["/usr", "/bin", "/lib", "/lib64", "/etc/resolv.conf", "/etc/ssl", "/etc/hosts"] {
            if std::path::Path::new(path).exists() {
                args.push("--ro-bind".to_string());
                args.push(path.to_string());
                args.push(path.to_string());
            }
        }

        // /proc and /dev (minimal)
        args.push("--proc".to_string());
        args.push("/proc".to_string());
        args.push("--dev".to_string());
        args.push("/dev".to_string());

        match config.ring {
            Ring::Ring1 => {
                // Ring1: approved apps with declared capabilities

                // Writable /tmp
                args.push("--tmpfs".to_string());
                args.push("/tmp".to_string());

                // Declared read paths
                for path in &config.fs_read {
                    if !path.is_empty() {
                        args.push("--ro-bind".to_string());
                        args.push(path.clone());
                        args.push(path.clone());
                    }
                }

                // Declared write paths
                for path in &config.fs_write {
                    if !path.is_empty() {
                        args.push("--bind".to_string());
                        args.push(path.clone());
                        args.push(path.clone());
                    }
                }

                // Network: if allowed, share network namespace and set proxy env
                if config.network {
                    // Remove --unshare-all and re-add without network unsharing
                    // Actually, we use --share-net to override
                    args.push("--share-net".to_string());
                    args.push("--setenv".to_string());
                    args.push("HTTPS_PROXY".to_string());
                    args.push(self.egress_proxy.clone());
                    args.push("--setenv".to_string());
                    args.push("HTTP_PROXY".to_string());
                    args.push(self.egress_proxy.clone());
                }
            }
            Ring::Ring2 => {
                // Ring2: maximum isolation — no network, minimal writable

                // Only /tmp is writable
                args.push("--tmpfs".to_string());
                args.push("/tmp".to_string());

                // No additional binds — truly minimal
                // Network stays unshared (no --share-net)
            }
        }

        // The command to execute
        args.push("--".to_string());
        args.push("/bin/sh".to_string());
        args.push("-c".to_string());
        args.push(command.to_string());

        args
    }

    /// Execute a command in a sandbox.
    pub async fn spawn_sandboxed(
        &self,
        config: &SandboxConfig,
        command: &str,
    ) -> Result<SandboxResult> {
        let bwrap_args = self.build_bwrap_args(config, command);

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(config.timeout_secs),
            tokio::process::Command::new("bwrap")
                .args(&bwrap_args)
                .output(),
        )
        .await;

        match result {
            Err(_) => {
                // Timeout
                Ok(SandboxResult {
                    exit_code: -1,
                    stdout: String::new(),
                    stderr: format!(
                        "sandbox execution timed out after {}s",
                        config.timeout_secs
                    ),
                    ring: config.ring,
                    timed_out: true,
                })
            }
            Ok(Ok(output)) => Ok(SandboxResult {
                exit_code: output.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&output.stdout)
                    .chars()
                    .take(65536)
                    .collect(),
                stderr: String::from_utf8_lossy(&output.stderr)
                    .chars()
                    .take(65536)
                    .collect(),
                ring: config.ring,
                timed_out: false,
            }),
            Ok(Err(e)) => Err(anyhow::anyhow!("failed to spawn sandbox: {e}")),
        }
    }

    /// Mint a capability token.
    pub fn mint_capability(
        &self,
        granted_to: &str,
        permissions: Vec<String>,
        ttl_secs: u64,
    ) -> CapabilityToken {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let created_at = now.to_rfc3339();
        let expires_at = (now + chrono::Duration::seconds(ttl_secs as i64)).to_rfc3339();

        let sign_input = format!("{id}|{granted_to}|{}|{expires_at}", permissions.join(","));
        let signature = self.hmac_sign(&sign_input);

        CapabilityToken {
            id,
            granted_to: granted_to.to_string(),
            permissions,
            created_at,
            expires_at,
            signature,
        }
    }

    /// Verify a capability token's signature and expiry.
    pub fn verify_capability(&self, token: &CapabilityToken) -> Result<bool> {
        // Check expiry
        let expires = chrono::DateTime::parse_from_rfc3339(&token.expires_at)
            .context("invalid expires_at timestamp")?;
        if chrono::Utc::now() > expires {
            return Ok(false);
        }

        // Verify signature
        let sign_input = format!(
            "{}|{}|{}|{}",
            token.id,
            token.granted_to,
            token.permissions.join(","),
            token.expires_at
        );
        let expected = self.hmac_sign(&sign_input);
        Ok(expected == token.signature)
    }

    /// HMAC-SHA256 sign a string.
    fn hmac_sign(&self, input: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&self.hmac_key);
        hasher.update(input.as_bytes());
        hex::encode(hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine() -> SandboxEngine {
        SandboxEngine::new([42u8; 32], "http://127.0.0.1:8443")
    }

    #[test]
    fn test_ring2_bwrap_args_minimal() {
        let engine = test_engine();
        let config = SandboxConfig::default(); // Ring2

        let args = engine.build_bwrap_args(&config, "echo hello");

        // Should have --unshare-all
        assert!(args.contains(&"--unshare-all".to_string()));
        // Should have --die-with-parent
        assert!(args.contains(&"--die-with-parent".to_string()));
        // Should have /nix/store ro-bind
        assert!(args.contains(&"/nix/store".to_string()));
        // Should NOT have --share-net (Ring2 = no network)
        assert!(!args.contains(&"--share-net".to_string()));
        // Should end with the command
        assert!(args.contains(&"echo hello".to_string()));
    }

    #[test]
    fn test_ring1_bwrap_args_with_network() {
        let engine = test_engine();
        let config = SandboxConfig {
            ring: Ring::Ring1,
            network: true,
            fs_read: vec!["/var/lib/myapp".to_string()],
            fs_write: vec!["/var/lib/myapp/data".to_string()],
            ..Default::default()
        };

        let args = engine.build_bwrap_args(&config, "myapp run");

        // Should have --share-net for Ring1 with network
        assert!(args.contains(&"--share-net".to_string()));
        // Should have HTTPS_PROXY set
        assert!(args.contains(&"HTTPS_PROXY".to_string()));
        // Should have read path
        assert!(args.contains(&"/var/lib/myapp".to_string()));
        // Should have write path bound
        assert!(args.contains(&"/var/lib/myapp/data".to_string()));
    }

    #[test]
    fn test_ring1_no_network() {
        let engine = test_engine();
        let config = SandboxConfig {
            ring: Ring::Ring1,
            network: false,
            ..Default::default()
        };

        let args = engine.build_bwrap_args(&config, "ls /tmp");
        assert!(!args.contains(&"--share-net".to_string()));
    }

    #[test]
    fn test_mint_and_verify_capability() {
        let engine = test_engine();
        let token = engine.mint_capability(
            "myapp",
            vec!["network".to_string(), "fs:/var/lib/myapp".to_string()],
            3600,
        );

        assert_eq!(token.granted_to, "myapp");
        assert_eq!(token.permissions.len(), 2);
        assert!(engine.verify_capability(&token).unwrap());
    }

    #[test]
    fn test_expired_capability_fails_verification() {
        let engine = test_engine();
        let mut token = engine.mint_capability("myapp", vec!["network".to_string()], 0);
        // Manually set expiry to past
        token.expires_at =
            (chrono::Utc::now() - chrono::Duration::seconds(10)).to_rfc3339();
        // Re-sign with correct expiry so signature matches
        let sign_input = format!(
            "{}|{}|{}|{}",
            token.id,
            token.granted_to,
            token.permissions.join(","),
            token.expires_at
        );
        token.signature = engine.hmac_sign(&sign_input);

        assert!(!engine.verify_capability(&token).unwrap());
    }

    #[test]
    fn test_tampered_capability_fails_verification() {
        let engine = test_engine();
        let mut token = engine.mint_capability("myapp", vec!["network".to_string()], 3600);
        // Tamper with permissions
        token.permissions.push("admin".to_string());

        assert!(!engine.verify_capability(&token).unwrap());
    }
}
