use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;
use tokio::sync::Mutex;

use crate::server::{self, ServerStatus};
use crate::McpdState;

type SharedState = Arc<Mutex<McpdState>>;

// ── GET /health ──

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub server_count: usize,
    pub running_count: usize,
    pub servers: Vec<ServerHealthInfo>,
}

#[derive(Debug, Serialize)]
pub struct ServerHealthInfo {
    pub name: String,
    pub status: ServerStatus,
    pub pid: Option<u32>,
    pub uptime: Option<String>,
}

pub async fn health_handler(State(state): State<SharedState>) -> Json<HealthResponse> {
    let st = state.lock().await;
    let now = chrono::Utc::now();

    let servers: Vec<ServerHealthInfo> = st
        .servers
        .iter()
        .map(|s| {
            let uptime = s.started_at.as_ref().and_then(|started| {
                chrono::DateTime::parse_from_rfc3339(started)
                    .ok()
                    .map(|dt| {
                        let dur = now.signed_duration_since(dt);
                        format!("{}s", dur.num_seconds())
                    })
            });
            ServerHealthInfo {
                name: s.config.name.clone(),
                status: s.status,
                pid: s.pid,
                uptime,
            }
        })
        .collect();

    let running = st
        .servers
        .iter()
        .filter(|s| matches!(s.status, ServerStatus::Running))
        .count();

    Json(HealthResponse {
        status: "ok".to_string(),
        server_count: st.servers.len(),
        running_count: running,
        servers,
    })
}

// ── GET /servers ──

#[derive(Debug, Serialize)]
pub struct ServerListEntry {
    pub name: String,
    pub command: String,
    pub transport: String,
    pub status: ServerStatus,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub restart_count: u32,
    pub last_error: Option<String>,
    pub allowed_domains: Vec<String>,
}

pub async fn servers_list_handler(State(state): State<SharedState>) -> Json<Vec<ServerListEntry>> {
    let st = state.lock().await;
    let entries: Vec<ServerListEntry> = st
        .servers
        .iter()
        .map(|s| ServerListEntry {
            name: s.config.name.clone(),
            command: s.config.command.clone(),
            transport: s.config.transport.clone(),
            status: s.status,
            pid: s.pid,
            started_at: s.started_at.clone(),
            restart_count: s.restart_count,
            last_error: s.last_error.clone(),
            allowed_domains: s.config.allowed_domains.clone(),
        })
        .collect();
    Json(entries)
}

// ── GET /server/{name} ──

pub async fn server_detail_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<ServerListEntry>, axum::http::StatusCode> {
    let st = state.lock().await;
    let srv = st
        .servers
        .iter()
        .find(|s| s.config.name == name)
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;

    Ok(Json(ServerListEntry {
        name: srv.config.name.clone(),
        command: srv.config.command.clone(),
        transport: srv.config.transport.clone(),
        status: srv.status,
        pid: srv.pid,
        started_at: srv.started_at.clone(),
        restart_count: srv.restart_count,
        last_error: srv.last_error.clone(),
        allowed_domains: srv.config.allowed_domains.clone(),
    }))
}

// ── POST /server/{name}/start ──

pub async fn server_start_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let mut st = state.lock().await;
    let egress_port = st.egress_port;
    let srv = st
        .servers
        .iter_mut()
        .find(|s| s.config.name == name)
        .ok_or((
            axum::http::StatusCode::NOT_FOUND,
            "server not found".to_string(),
        ))?;

    if matches!(srv.status, ServerStatus::Running) {
        return Ok(Json(
            serde_json::json!({"status": "already_running", "name": name}),
        ));
    }

    server::start_server(srv, egress_port)
        .await
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
            )
        })?;

    st.receipt_logger
        .log_event("server.start", &name, "started via API")
        .await;

    Ok(Json(serde_json::json!({"status": "started", "name": name})))
}

// ── POST /server/{name}/stop ──

pub async fn server_stop_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let mut st = state.lock().await;
    let srv = st
        .servers
        .iter_mut()
        .find(|s| s.config.name == name)
        .ok_or((
            axum::http::StatusCode::NOT_FOUND,
            "server not found".to_string(),
        ))?;

    server::stop_server(srv).await.map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        )
    })?;

    st.receipt_logger
        .log_event("server.stop", &name, "stopped via API")
        .await;

    Ok(Json(serde_json::json!({"status": "stopped", "name": name})))
}

// ── POST /server/{name}/restart ──

pub async fn server_restart_handler(
    State(state): State<SharedState>,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let mut st = state.lock().await;
    let egress_port = st.egress_port;
    let srv = st
        .servers
        .iter_mut()
        .find(|s| s.config.name == name)
        .ok_or((
            axum::http::StatusCode::NOT_FOUND,
            "server not found".to_string(),
        ))?;

    server::stop_server(srv).await.map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        )
    })?;

    server::start_server(srv, egress_port)
        .await
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
            )
        })?;

    srv.restart_count += 1;

    st.receipt_logger
        .log_event("server.restart", &name, "restarted via API")
        .await;

    Ok(Json(
        serde_json::json!({"status": "restarted", "name": name}),
    ))
}

// ── POST /reload ──

pub async fn reload_handler(
    State(state): State<SharedState>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let mut st = state.lock().await;
    let config_path = st.config_path.clone();
    let egress_port = st.egress_port;
    let output_path = st.output_config_path.clone();

    // Load new config
    let new_configs = crate::load_server_configs(&config_path);
    let new_names: Vec<String> = new_configs.iter().map(|c| c.name.clone()).collect();
    let existing_names: Vec<String> = st.servers.iter().map(|s| s.config.name.clone()).collect();

    // Stop servers that are no longer in config
    let to_remove: Vec<String> = existing_names
        .iter()
        .filter(|n| !new_names.contains(n))
        .cloned()
        .collect();

    for name in &to_remove {
        if let Some(srv) = st.servers.iter_mut().find(|s| &s.config.name == name) {
            server::stop_server(srv).await.ok();
            st.receipt_logger
                .log_event("server.stop", name, "removed during reload")
                .await;
        }
    }
    st.servers.retain(|s| !to_remove.contains(&s.config.name));

    // Start new servers
    let mut started = 0;
    for config in new_configs {
        if !existing_names.contains(&config.name) {
            let mut srv = server::ManagedServer::from_config(config);
            if let Err(e) = server::start_server(&mut srv, egress_port).await {
                tracing::error!(server = %srv.config.name, error = %e, "failed to start new MCP server during reload");
            } else {
                st.receipt_logger
                    .log_event("server.start", &srv.config.name, "started during reload")
                    .await;
                started += 1;
            }
            st.servers.push(srv);
        }
    }

    // Rewrite OpenClaw config
    if let Err(e) = server::write_openclaw_config(&st.servers, &output_path, egress_port) {
        tracing::error!(error = %e, "failed to rewrite OpenClaw config during reload");
    }

    Ok(Json(serde_json::json!({
        "status": "reloaded",
        "removed": to_remove.len(),
        "started": started,
        "total": st.servers.len(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ServerStatus;

    #[test]
    fn test_health_response_structure() {
        let health = HealthResponse {
            status: "ok".to_string(),
            server_count: 2,
            running_count: 1,
            servers: vec![
                ServerHealthInfo {
                    name: "server-a".to_string(),
                    status: ServerStatus::Running,
                    pid: Some(1234),
                    uptime: Some("120s".to_string()),
                },
                ServerHealthInfo {
                    name: "server-b".to_string(),
                    status: ServerStatus::Stopped,
                    pid: None,
                    uptime: None,
                },
            ],
        };

        let json = serde_json::to_value(&health).expect("serialize");
        assert_eq!(json["status"], "ok");
        assert_eq!(json["server_count"], 2);
        assert_eq!(json["running_count"], 1);
        assert!(json["servers"].is_array());
        assert_eq!(json["servers"][0]["name"], "server-a");
        assert_eq!(json["servers"][0]["status"], "running");
        assert_eq!(json["servers"][1]["status"], "stopped");
    }

    #[test]
    fn test_server_list_entry_serialization() {
        let entry = ServerListEntry {
            name: "test-mcp".to_string(),
            command: "node".to_string(),
            transport: "stdio".to_string(),
            status: ServerStatus::Running,
            pid: Some(5678),
            started_at: Some("2026-01-01T00:00:00Z".to_string()),
            restart_count: 2,
            last_error: None,
            allowed_domains: vec!["api.example.com".to_string()],
        };

        let json = serde_json::to_value(&entry).expect("serialize");
        assert_eq!(json["name"], "test-mcp");
        assert_eq!(json["status"], "running");
        assert_eq!(json["restart_count"], 2);
        assert!(json["last_error"].is_null());
    }
}
