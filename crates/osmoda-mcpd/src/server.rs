use std::collections::HashMap;
use std::process::Stdio;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::process::Child;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default = "default_transport")]
    pub transport: String,
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    #[serde(default)]
    pub secret_file: Option<String>,
}

fn default_transport() -> String {
    "stdio".to_string()
}

#[derive(Debug)]
pub struct ManagedServer {
    pub config: ServerConfig,
    pub status: ServerStatus,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub restart_count: u32,
    pub last_error: Option<String>,
    pub child: Option<Child>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServerStatus {
    Stopped,
    Starting,
    Running,
    Failed,
    Restarting,
}

impl std::fmt::Display for ServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerStatus::Stopped => write!(f, "stopped"),
            ServerStatus::Starting => write!(f, "starting"),
            ServerStatus::Running => write!(f, "running"),
            ServerStatus::Failed => write!(f, "failed"),
            ServerStatus::Restarting => write!(f, "restarting"),
        }
    }
}

impl ManagedServer {
    pub fn from_config(config: ServerConfig) -> Self {
        Self {
            config,
            status: ServerStatus::Stopped,
            pid: None,
            started_at: None,
            restart_count: 0,
            last_error: None,
            child: None,
        }
    }
}

/// Start an MCP server as a child process.
pub async fn start_server(server: &mut ManagedServer, egress_port: u16) -> Result<()> {
    server.status = ServerStatus::Starting;

    let mut env = server.config.env.clone();

    // If allowed_domains is set, inject HTTP_PROXY so traffic routes through osmoda-egress
    if !server.config.allowed_domains.is_empty() {
        env.insert(
            "HTTP_PROXY".to_string(),
            format!("http://127.0.0.1:{}", egress_port),
        );
        env.insert(
            "HTTPS_PROXY".to_string(),
            format!("http://127.0.0.1:{}", egress_port),
        );
    }

    // If secret_file is set, read it and inject as an env var
    if let Some(ref secret_path) = server.config.secret_file {
        match std::fs::read_to_string(secret_path) {
            Ok(secret) => {
                let env_key = format!(
                    "{}_SECRET",
                    server.config.name.to_uppercase().replace('-', "_")
                );
                env.insert(env_key, secret.trim().to_string());
            }
            Err(e) => {
                tracing::warn!(
                    server = %server.config.name,
                    path = %secret_path,
                    error = %e,
                    "failed to read secret file"
                );
            }
        }
    }

    let child = tokio::process::Command::new(&server.config.command)
        .args(&server.config.args)
        .envs(&env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    server.pid = Some(child.id().unwrap_or(0));
    server.started_at = Some(chrono::Utc::now().to_rfc3339());
    server.status = ServerStatus::Running;
    server.last_error = None;
    server.child = Some(child);

    tracing::info!(
        server = %server.config.name,
        pid = ?server.pid,
        "MCP server started"
    );

    Ok(())
}

/// Stop an MCP server by killing its child process.
pub async fn stop_server(server: &mut ManagedServer) -> Result<()> {
    if let Some(ref mut child) = server.child {
        child.kill().await.ok();
        child.wait().await.ok();
    }
    server.child = None;
    server.pid = None;
    server.status = ServerStatus::Stopped;
    tracing::info!(server = %server.config.name, "MCP server stopped");
    Ok(())
}

/// Check if a running server's process is still alive.
pub fn check_server(server: &mut ManagedServer) {
    if let Some(ref mut child) = server.child {
        match child.try_wait() {
            Ok(Some(exit_status)) => {
                // Process has exited
                server.status = ServerStatus::Failed;
                server.last_error = Some(format!("process exited with {}", exit_status));
                server.child = None;
                server.pid = None;
            }
            Ok(None) => {
                // Still running
            }
            Err(e) => {
                server.status = ServerStatus::Failed;
                server.last_error = Some(format!("failed to check process: {}", e));
            }
        }
    } else if matches!(server.status, ServerStatus::Running) {
        // No child but status says running — mark as failed
        server.status = ServerStatus::Failed;
        server.last_error = Some("child process handle lost".to_string());
    }
}

/// Generate OpenClaw MCP servers config JSON.
pub fn generate_openclaw_config(
    servers: &[ManagedServer],
    egress_port: u16,
) -> serde_json::Value {
    let mut mcp_servers = serde_json::Map::new();

    for srv in servers {
        if !matches!(srv.status, ServerStatus::Running | ServerStatus::Starting) {
            continue;
        }

        let mut env = serde_json::Map::new();
        for (k, v) in &srv.config.env {
            env.insert(k.clone(), serde_json::Value::String(v.clone()));
        }

        // Inject proxy env for servers with domain restrictions
        if !srv.config.allowed_domains.is_empty() {
            let proxy = format!("http://127.0.0.1:{}", egress_port);
            env.insert(
                "HTTP_PROXY".to_string(),
                serde_json::Value::String(proxy.clone()),
            );
            env.insert(
                "HTTPS_PROXY".to_string(),
                serde_json::Value::String(proxy),
            );
        }

        let entry = serde_json::json!({
            "command": srv.config.command,
            "args": srv.config.args,
            "env": env,
            "transport": srv.config.transport,
        });

        mcp_servers.insert(srv.config.name.clone(), entry);
    }

    serde_json::Value::Object(mcp_servers)
}

/// Write the OpenClaw MCP config to disk.
pub fn write_openclaw_config(
    servers: &[ManagedServer],
    output_path: &str,
    egress_port: u16,
) -> Result<()> {
    let config = generate_openclaw_config(servers, egress_port);
    let json = serde_json::to_string_pretty(&config)?;

    if let Some(parent) = std::path::Path::new(output_path).parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(output_path, json)?;
    tracing::info!(path = %output_path, "wrote OpenClaw MCP config");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_server_config(name: &str) -> ServerConfig {
        ServerConfig {
            name: name.to_string(),
            command: "/usr/bin/echo".to_string(),
            args: vec!["hello".to_string()],
            env: HashMap::new(),
            transport: "stdio".to_string(),
            allowed_domains: vec![],
            secret_file: None,
        }
    }

    #[test]
    fn test_server_config_serde() {
        let config = ServerConfig {
            name: "test-server".to_string(),
            command: "node".to_string(),
            args: vec!["server.js".to_string()],
            env: HashMap::from([("API_KEY".to_string(), "secret".to_string())]),
            transport: "stdio".to_string(),
            allowed_domains: vec!["api.example.com".to_string()],
            secret_file: Some("/run/secrets/test".to_string()),
        };

        let json = serde_json::to_string(&config).expect("serialize");
        let parsed: ServerConfig = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(parsed.name, "test-server");
        assert_eq!(parsed.command, "node");
        assert_eq!(parsed.args, vec!["server.js"]);
        assert_eq!(parsed.env.get("API_KEY").unwrap(), "secret");
        assert_eq!(parsed.transport, "stdio");
        assert_eq!(parsed.allowed_domains, vec!["api.example.com"]);
        assert_eq!(parsed.secret_file, Some("/run/secrets/test".to_string()));
    }

    #[test]
    fn test_generate_openclaw_config() {
        let servers = vec![
            ManagedServer {
                config: make_server_config("server-a"),
                status: ServerStatus::Running,
                pid: Some(1234),
                started_at: Some("2026-01-01T00:00:00Z".to_string()),
                restart_count: 0,
                last_error: None,
                child: None,
            },
            ManagedServer {
                config: make_server_config("server-b"),
                status: ServerStatus::Stopped,
                pid: None,
                started_at: None,
                restart_count: 0,
                last_error: None,
                child: None,
            },
        ];

        let config = generate_openclaw_config(&servers, 19999);
        let obj = config.as_object().expect("should be object");

        // Only running server should be in config
        assert!(obj.contains_key("server-a"), "running server should be present");
        assert!(!obj.contains_key("server-b"), "stopped server should not be present");

        let entry = &obj["server-a"];
        assert_eq!(entry["command"], "/usr/bin/echo");
        assert_eq!(entry["transport"], "stdio");
    }

    #[test]
    fn test_generate_openclaw_config_with_proxy() {
        let mut config = make_server_config("proxy-server");
        config.allowed_domains = vec!["api.example.com".to_string()];

        let servers = vec![ManagedServer {
            config,
            status: ServerStatus::Running,
            pid: Some(5678),
            started_at: Some("2026-01-01T00:00:00Z".to_string()),
            restart_count: 0,
            last_error: None,
            child: None,
        }];

        let oc_config = generate_openclaw_config(&servers, 19999);
        let entry = &oc_config["proxy-server"];
        let env = entry["env"].as_object().expect("env should be object");

        assert_eq!(
            env.get("HTTP_PROXY").unwrap(),
            "http://127.0.0.1:19999"
        );
        assert_eq!(
            env.get("HTTPS_PROXY").unwrap(),
            "http://127.0.0.1:19999"
        );
    }

    #[test]
    fn test_generate_openclaw_config_empty() {
        let servers: Vec<ManagedServer> = vec![];
        let config = generate_openclaw_config(&servers, 19999);
        let obj = config.as_object().expect("should be object");
        assert!(obj.is_empty(), "empty servers should produce empty config");
    }

    #[test]
    fn test_managed_server_status_transitions() {
        let config = make_server_config("test");
        let mut server = ManagedServer::from_config(config);

        assert!(matches!(server.status, ServerStatus::Stopped));

        server.status = ServerStatus::Starting;
        assert!(matches!(server.status, ServerStatus::Starting));

        server.status = ServerStatus::Running;
        assert!(matches!(server.status, ServerStatus::Running));

        server.status = ServerStatus::Failed;
        assert!(matches!(server.status, ServerStatus::Failed));

        server.status = ServerStatus::Restarting;
        assert!(matches!(server.status, ServerStatus::Restarting));

        server.status = ServerStatus::Running;
        assert!(matches!(server.status, ServerStatus::Running));
    }

    #[test]
    fn test_server_config_default_transport() {
        let json = r#"{"name": "test", "command": "echo", "args": []}"#;
        let config: ServerConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(config.transport, "stdio");
    }
}
