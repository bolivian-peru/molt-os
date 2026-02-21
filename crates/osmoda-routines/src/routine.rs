use std::time::Duration;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routine {
    pub id: String,
    pub name: String,
    pub trigger: Trigger,
    pub action: RoutineAction,
    pub enabled: bool,
    pub last_run: Option<String>,
    pub run_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    Cron { expression: String },
    Interval { seconds: u64 },
    Event { event_type: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoutineAction {
    HealthCheck,
    ServiceMonitor { units: Vec<String> },
    LogScan { priority: String },
    MemoryMaintenance,
    Command { cmd: String, args: Vec<String> },
    Webhook { url: String, method: String, body: Option<String> },
}

/// Validate that a command path is allowed. Rejects shell interpreters and
/// requires absolute paths to prevent ambient PATH abuse.
pub fn validate_command(cmd: &str) -> Result<(), String> {
    // Must be an absolute path
    if !cmd.starts_with('/') {
        return Err(format!("command must be an absolute path, got: {cmd}"));
    }

    // Block shell interpreters — these allow arbitrary code execution via -c
    const BLOCKED_BINARIES: &[&str] = &[
        "/bin/sh", "/bin/bash", "/bin/zsh", "/bin/dash", "/bin/fish", "/bin/csh", "/bin/tcsh",
        "/usr/bin/sh", "/usr/bin/bash", "/usr/bin/zsh", "/usr/bin/dash", "/usr/bin/fish",
        "/usr/bin/env", "/usr/bin/python", "/usr/bin/python3", "/usr/bin/perl", "/usr/bin/ruby",
        "/usr/bin/node", "/usr/bin/lua",
        // NixOS paths
        "/run/current-system/sw/bin/sh", "/run/current-system/sw/bin/bash",
        "/run/current-system/sw/bin/zsh", "/run/current-system/sw/bin/env",
        "/run/current-system/sw/bin/python", "/run/current-system/sw/bin/python3",
        "/run/current-system/sw/bin/perl", "/run/current-system/sw/bin/ruby",
        "/run/current-system/sw/bin/node",
    ];

    // Normalize by resolving the basename for /nix/store paths
    let basename = cmd.rsplit('/').next().unwrap_or(cmd);
    let blocked_basenames = ["sh", "bash", "zsh", "dash", "fish", "csh", "tcsh",
                             "env", "python", "python3", "perl", "ruby", "node", "lua"];

    if BLOCKED_BINARIES.contains(&cmd) || blocked_basenames.contains(&basename) {
        return Err(format!("shell interpreters are blocked for security: {cmd}"));
    }

    // Block path traversal
    if cmd.contains("..") {
        return Err("command path must not contain '..'".to_string());
    }

    Ok(())
}

/// Validate that a webhook URL uses an allowed scheme (http/https only).
pub fn validate_webhook_url(url: &str) -> Result<(), String> {
    let lower = url.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        Ok(())
    } else {
        Err(format!(
            "webhook URL must use http:// or https:// scheme, got: {}",
            url.split("://").next().unwrap_or("unknown")
        ))
    }
}

/// Validate a systemd unit name — only safe characters allowed.
pub fn validate_unit_name(unit: &str) -> Result<(), String> {
    if unit.is_empty() || unit.len() > 256 {
        return Err("unit name must be 1-256 characters".to_string());
    }
    if !unit.chars().all(|c| c.is_ascii_alphanumeric() || c == '@' || c == '.' || c == '_' || c == '-') {
        return Err(format!("invalid characters in unit name: {unit}"));
    }
    Ok(())
}

impl Routine {
    pub fn should_run(&self, now: &chrono::DateTime<chrono::Utc>) -> bool {
        if !self.enabled {
            return false;
        }

        match &self.trigger {
            Trigger::Interval { seconds } => {
                if let Some(ref last) = self.last_run {
                    if let Ok(last_dt) = chrono::DateTime::parse_from_rfc3339(last) {
                        let elapsed = now.signed_duration_since(last_dt);
                        return elapsed.num_seconds() as u64 >= *seconds;
                    }
                }
                true // never run before
            }
            Trigger::Cron { expression } => {
                crate::scheduler::cron_matches(expression, now)
            }
            Trigger::Event { .. } => {
                false // event-based triggers don't run on schedule
            }
        }
    }
}

/// Execute a routine action. Returns output string.
pub async fn execute_action(action: &RoutineAction) -> Result<String, String> {
    match action {
        RoutineAction::HealthCheck => {
            // Simple health check — run system health via shell
            let output = tokio::time::timeout(
                Duration::from_secs(10),
                tokio::process::Command::new("systemctl")
                    .args(["is-system-running"])
                    .output()
            ).await
            .map_err(|_| "health check timed out after 10s".to_string())?
            .map_err(|e| format!("health check failed: {e}"))?;
            let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(format!("system status: {status}"))
        }
        RoutineAction::ServiceMonitor { units } => {
            let mut results = Vec::new();
            for unit in units {
                let output = tokio::time::timeout(
                    Duration::from_secs(10),
                    tokio::process::Command::new("systemctl")
                        .args(["is-active", unit])
                        .output()
                ).await
                .map_err(|_| format!("{unit} check timed out after 10s"))?
                .map_err(|e| format!("failed to check {unit}: {e}"))?;
                let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
                results.push(format!("{unit}: {status}"));
            }
            Ok(results.join(", "))
        }
        RoutineAction::LogScan { priority } => {
            let output = tokio::time::timeout(
                Duration::from_secs(15),
                tokio::process::Command::new("journalctl")
                    .args(["--no-pager", "-p", priority, "--since", "15 minutes ago", "-n", "20"])
                    .output()
            ).await
            .map_err(|_| "log scan timed out after 15s".to_string())?
            .map_err(|e| format!("log scan failed: {e}"))?;
            let logs = String::from_utf8_lossy(&output.stdout).to_string();
            Ok(if logs.trim().is_empty() {
                "no matching log entries".to_string()
            } else {
                logs
            })
        }
        RoutineAction::MemoryMaintenance => {
            memory_maintenance().await
        }
        RoutineAction::Command { cmd, args } => {
            validate_command(cmd)?;
            let output = tokio::time::timeout(
                Duration::from_secs(30),
                tokio::process::Command::new(cmd)
                    .args(args)
                    .output()
            ).await
            .map_err(|_| "command timed out after 30s".to_string())?
            .map_err(|e| format!("command failed: {e}"))?;
            if output.status.success() {
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            } else {
                Err(String::from_utf8_lossy(&output.stderr).to_string())
            }
        }
        RoutineAction::Webhook { url, method, body } => {
            validate_webhook_url(url)?;
            let mut cmd_args = vec!["-s", "--proto", "=http,https", "--max-time", "10", "-X", method.as_str(), url.as_str()];
            let body_owned;
            if let Some(b) = body {
                cmd_args.push("-d");
                body_owned = b.clone();
                cmd_args.push(&body_owned);
                cmd_args.push("-H");
                cmd_args.push("Content-Type: application/json");
            }
            let output = tokio::time::timeout(
                Duration::from_secs(15),
                tokio::process::Command::new("curl")
                    .args(&cmd_args)
                    .output()
            ).await
            .map_err(|_| "webhook timed out after 15s".to_string())?
            .map_err(|e| format!("webhook failed: {e}"))?;
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
    }
}

/// Consolidate recent ledger events into a memory summary.
/// Fetches recent events from agentd, counts by type, and stores a summary.
async fn memory_maintenance() -> Result<String, String> {
    let socket = "/run/osmoda/agentd.sock";

    // Fetch recent events from agentd
    let events_json = agentd_get(socket, "/events/log?limit=50").await.map_err(|e| {
        format!("failed to fetch events: {e}")
    })?;

    let events: Vec<serde_json::Value> =
        serde_json::from_str(&events_json).unwrap_or_default();

    if events.is_empty() {
        return Ok("no recent events to consolidate".to_string());
    }

    // Count events by type
    let mut type_counts: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    for event in &events {
        if let Some(t) = event.get("type").and_then(|v| v.as_str()) {
            *type_counts.entry(t.to_string()).or_insert(0) += 1;
        }
    }

    let summary = format!(
        "Memory maintenance: consolidated {} recent events. Types: {}",
        events.len(),
        type_counts
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Store summary in agentd memory
    let store_body = serde_json::json!({
        "summary": summary,
        "detail": format!("Event consolidation at {}", chrono::Utc::now().to_rfc3339()),
        "category": "maintenance",
        "tags": ["memory", "maintenance", "routine"]
    });

    let _ = agentd_post(socket, "/memory/store", &store_body.to_string()).await;

    Ok(summary)
}

/// HTTP GET over Unix socket to agentd.
async fn agentd_get(socket_path: &str, path: &str) -> Result<String, String> {
    use hyper::body::Bytes;
    use hyper::Request;
    use hyper_util::rt::TokioIo;
    use http_body_util::{Empty, BodyExt};
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| format!("handshake failed: {e}"))?;
    tokio::spawn(async move { let _ = conn.await; });

    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Empty::<Bytes>::new())
        .map_err(|e| format!("request build failed: {e}"))?;

    let resp = sender.send_request(req).await.map_err(|e| format!("request failed: {e}"))?;
    let body = resp.into_body().collect().await.map_err(|e| format!("body read failed: {e}"))?;
    Ok(String::from_utf8_lossy(&body.to_bytes()).to_string())
}

/// HTTP POST over Unix socket to agentd.
async fn agentd_post(socket_path: &str, path: &str, body: &str) -> Result<String, String> {
    use hyper::body::Bytes;
    use hyper::Request;
    use hyper_util::rt::TokioIo;
    use http_body_util::{Full, BodyExt};
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| format!("connect failed: {e}"))?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| format!("handshake failed: {e}"))?;
    tokio::spawn(async move { let _ = conn.await; });

    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .map_err(|e| format!("request build failed: {e}"))?;

    let resp = sender.send_request(req).await.map_err(|e| format!("request failed: {e}"))?;
    let body = resp.into_body().collect().await.map_err(|e| format!("body read failed: {e}"))?;
    Ok(String::from_utf8_lossy(&body.to_bytes()).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_command_requires_absolute_path() {
        assert!(validate_command("relative/path").is_err());
        assert!(validate_command("just-a-name").is_err());
        assert!(validate_command("/usr/bin/systemctl").is_ok());
    }

    #[test]
    fn test_validate_command_blocks_interpreters() {
        assert!(validate_command("/bin/sh").is_err());
        assert!(validate_command("/usr/bin/bash").is_err());
        assert!(validate_command("/usr/bin/python3").is_err());
        assert!(validate_command("/usr/bin/env").is_err());
        assert!(validate_command("/run/current-system/sw/bin/bash").is_err());
        // NixOS store paths are caught by basename check
        assert!(validate_command("/nix/store/abc123-bash/bin/bash").is_err());
    }

    #[test]
    fn test_validate_command_blocks_path_traversal() {
        assert!(validate_command("/usr/bin/../bin/sh").is_err());
    }

    #[test]
    fn test_validate_command_allows_safe_commands() {
        assert!(validate_command("/usr/bin/systemctl").is_ok());
        assert!(validate_command("/run/current-system/sw/bin/nixos-rebuild").is_ok());
        assert!(validate_command("/usr/bin/curl").is_ok());
    }

    #[test]
    fn test_validate_webhook_url_valid() {
        assert!(validate_webhook_url("http://localhost:8080/hook").is_ok());
        assert!(validate_webhook_url("https://example.com/api").is_ok());
    }

    #[test]
    fn test_validate_webhook_url_rejects_dangerous_schemes() {
        assert!(validate_webhook_url("file:///etc/passwd").is_err());
        assert!(validate_webhook_url("gopher://evil.com").is_err());
        assert!(validate_webhook_url("ftp://server/file").is_err());
    }

    #[test]
    fn test_validate_unit_name_valid() {
        assert!(validate_unit_name("sshd").is_ok());
        assert!(validate_unit_name("osmoda-agentd.service").is_ok());
        assert!(validate_unit_name("foo@bar.service").is_ok());
    }

    #[test]
    fn test_validate_unit_name_rejects_injection() {
        assert!(validate_unit_name("").is_err());
        assert!(validate_unit_name("foo; rm -rf /").is_err());
        assert!(validate_unit_name("foo$(whoami)").is_err());
        assert!(validate_unit_name("foo`id`").is_err());
    }

    #[tokio::test]
    async fn test_command_timeout() {
        let action = RoutineAction::Command {
            cmd: "/bin/sleep".to_string(),
            args: vec!["60".to_string()],
        };
        let result = execute_action(&action).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("timed out"));
    }
}

/// Create the default set of routines (matching HEARTBEAT.md).
pub fn default_routines() -> Vec<Routine> {
    vec![
        Routine {
            id: "default-health".to_string(),
            name: "Health Check".to_string(),
            trigger: Trigger::Interval { seconds: 300 },
            action: RoutineAction::HealthCheck,
            enabled: true,
            last_run: None,
            run_count: 0,
        },
        Routine {
            id: "default-services".to_string(),
            name: "Service Monitor".to_string(),
            trigger: Trigger::Interval { seconds: 600 },
            action: RoutineAction::ServiceMonitor {
                units: vec![
                    "osmoda-agentd".to_string(),
                    "osmoda-gateway".to_string(),
                    "sshd".to_string(),
                ],
            },
            enabled: true,
            last_run: None,
            run_count: 0,
        },
        Routine {
            id: "default-logscan".to_string(),
            name: "Log Scan".to_string(),
            trigger: Trigger::Interval { seconds: 900 },
            action: RoutineAction::LogScan {
                priority: "err".to_string(),
            },
            enabled: true,
            last_run: None,
            run_count: 0,
        },
    ]
}
