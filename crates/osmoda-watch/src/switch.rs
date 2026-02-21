use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchSession {
    pub id: String,
    pub plan: String,
    pub ttl_secs: u64,
    pub health_checks: Vec<HealthCheck>,
    pub started_at: String,
    pub previous_generation: String,
    pub status: SwitchStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HealthCheck {
    SystemdUnit { unit: String },
    TcpPort { host: String, port: u16 },
    HttpGet { url: String, expect_status: u16 },
    Command { cmd: String, args: Vec<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SwitchStatus {
    Probation,
    Committed { committed_at: String },
    RolledBack { reason: String, rolled_back_at: String },
}

impl SwitchSession {
    pub fn is_active(&self) -> bool {
        matches!(self.status, SwitchStatus::Probation)
    }

    pub fn is_expired(&self) -> bool {
        if let Ok(started) = chrono::DateTime::parse_from_rfc3339(&self.started_at) {
            let elapsed = chrono::Utc::now().signed_duration_since(started);
            elapsed.num_seconds() as u64 >= self.ttl_secs
        } else {
            false
        }
    }
}

/// Run all health checks for a switch session. Returns (passed, failed_reasons).
pub async fn run_health_checks(checks: &[HealthCheck]) -> (bool, Vec<String>) {
    let mut failures = Vec::new();

    for check in checks {
        // Validate before executing (defense-in-depth — also validated at registration)
        if let Err(e) = crate::validate::validate_health_check(check) {
            failures.push(format!("invalid health check: {e}"));
            continue;
        }

        match check {
            HealthCheck::SystemdUnit { unit } => {
                let output = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tokio::process::Command::new("systemctl")
                        .args(["is-active", unit])
                        .output()
                ).await;
                match output {
                    Err(_) => {
                        failures.push(format!("systemd unit {unit} check timed out after 10s"));
                    }
                    Ok(Ok(o)) if o.status.success() => {}
                    Ok(Ok(o)) => {
                        let status = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        failures.push(format!("systemd unit {unit} is {status}"));
                    }
                    Ok(Err(e)) => {
                        failures.push(format!("failed to check unit {unit}: {e}"));
                    }
                }
            }
            HealthCheck::TcpPort { host, port } => {
                let result = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    tokio::net::TcpStream::connect(format!("{host}:{port}"))
                ).await;
                match result {
                    Err(_) => {
                        failures.push(format!("tcp {host}:{port} check timed out after 5s"));
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        failures.push(format!("tcp {host}:{port} unreachable: {e}"));
                    }
                }
            }
            HealthCheck::HttpGet { url, expect_status } => {
                let output = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tokio::process::Command::new("curl")
                        .args(["-s", "--proto", "=http,https", "--max-time", "5", "-o", "/dev/null", "-w", "%{http_code}", url])
                        .output()
                ).await;
                match output {
                    Err(_) => {
                        failures.push(format!("HTTP GET {url} timed out after 10s"));
                    }
                    Ok(Ok(o)) => {
                        let code_str = String::from_utf8_lossy(&o.stdout).trim().to_string();
                        let code: u16 = code_str.parse().unwrap_or(0);
                        if code != *expect_status {
                            failures.push(format!(
                                "HTTP GET {url}: expected {expect_status}, got {code}"
                            ));
                        }
                    }
                    Ok(Err(e)) => {
                        failures.push(format!("HTTP GET {url} failed: {e}"));
                    }
                }
            }
            HealthCheck::Command { cmd, args } => {
                let output = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    tokio::process::Command::new(cmd)
                        .args(args)
                        .output()
                ).await;
                match output {
                    Err(_) => {
                        failures.push(format!("command `{cmd}` timed out after 30s"));
                    }
                    Ok(Ok(o)) if o.status.success() => {}
                    Ok(Ok(o)) => {
                        let stderr = String::from_utf8_lossy(&o.stderr).trim().to_string();
                        failures.push(format!(
                            "command `{cmd}` exited {}: {stderr}",
                            o.status.code().unwrap_or(-1)
                        ));
                    }
                    Ok(Err(e)) => {
                        failures.push(format!("command `{cmd}` failed to execute: {e}"));
                    }
                }
            }
        }
    }

    let passed = failures.is_empty();
    (passed, failures)
}

/// Get the current NixOS system generation path.
pub fn current_generation() -> Result<String> {
    let target = std::fs::read_link("/nix/var/nix/profiles/system")
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    Ok(target)
}

/// Rollback to the previous NixOS generation.
pub async fn rollback_generation() -> Result<String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tokio::process::Command::new("nix-env")
            .args(["-p", "/nix/var/nix/profiles/system", "--rollback"])
            .output()
    ).await
    .map_err(|_| anyhow::anyhow!("nix-env rollback timed out after 60s"))?
    ?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("rollback failed: {stderr}");
    }

    // Switch to the rolled-back configuration
    let switch_output = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        tokio::process::Command::new("/nix/var/nix/profiles/system/bin/switch-to-configuration")
            .arg("switch")
            .output()
    ).await
    .map_err(|_| anyhow::anyhow!("switch-to-configuration timed out after 60s"))?
    ?;

    if !switch_output.status.success() {
        let stderr = String::from_utf8_lossy(&switch_output.stderr);
        anyhow::bail!("switch-to-configuration failed: {stderr}");
    }

    current_generation()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_switch_session_expiry() {
        let session = SwitchSession {
            id: "test".to_string(),
            plan: "test plan".to_string(),
            ttl_secs: 0, // immediately expired
            health_checks: vec![],
            started_at: chrono::Utc::now().to_rfc3339(),
            previous_generation: "/nix/store/test".to_string(),
            status: SwitchStatus::Probation,
        };
        assert!(session.is_active());
        assert!(session.is_expired());
    }

    #[test]
    fn test_switch_status_serialization() {
        let status = SwitchStatus::Committed {
            committed_at: "2026-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("committed"));
    }

    #[test]
    fn test_health_check_serialization() {
        let check = HealthCheck::SystemdUnit {
            unit: "sshd".to_string(),
        };
        let json = serde_json::to_string(&check).unwrap();
        assert!(json.contains("systemd_unit"));
        assert!(json.contains("sshd"));
    }
}
