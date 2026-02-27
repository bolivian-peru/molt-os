mod api;
mod switch;
pub mod validate;
mod watcher;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{delete, get, post};
use axum::Router;
use clap::Parser;
use tokio::net::UnixListener;
use tokio::signal;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::switch::{SwitchSession, SwitchStatus};
use crate::watcher::Watcher;

/// osModa Watch Daemon — SafeSwitch deploy transactions + autopilot health watchers.
#[derive(Parser, Debug)]
#[command(name = "osmoda-watch", version, about)]
struct Args {
    /// Path to the Unix domain socket to listen on.
    #[arg(long, default_value = "/run/osmoda/watch.sock")]
    socket: String,

    /// Path to the agentd Unix socket (for receipt logging).
    #[arg(long, default_value = "/run/osmoda/agentd.sock")]
    agentd_socket: String,

    /// Watcher check interval in seconds.
    #[arg(long, default_value_t = 30)]
    check_interval: u64,

    /// Directory for persisted watcher definitions.
    #[arg(long, default_value = "/var/lib/osmoda/watch")]
    data_dir: String,
}

pub struct WatchState {
    pub switches: HashMap<String, SwitchSession>,
    pub watchers: Vec<Watcher>,
    pub agentd_socket: String,
    pub data_dir: String,
}

/// POST an event to agentd /events/log over Unix socket. Best-effort — never blocks caller.
pub async fn agentd_post_event(socket_path: &str, event_type: &str, payload: &str) -> anyhow::Result<()> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::Request;
    use hyper_util::rt::TokioIo;
    use tokio::net::UnixStream;

    let body = serde_json::json!({
        "source": "osmoda-watch",
        "content": payload,
        "category": event_type,
        "tags": ["watch", event_type],
    });
    let body_str = serde_json::to_string(&body)?;

    let stream = UnixStream::connect(socket_path).await?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::debug!(error = %e, "agentd connection closed");
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/memory/ingest")
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body_str)))?;

    let _resp = sender.send_request(req).await?;
    Ok(())
}

#[tokio::main]
async fn main() {
    // SECURITY: restrict file creation permissions — no world/group access
    unsafe { libc::umask(0o077); }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    tracing::info!(socket = %args.socket, data_dir = %args.data_dir, "starting osmoda-watch");

    std::fs::create_dir_all(&args.data_dir).expect("failed to create watch data directory");

    let watchers = api::load_watchers(&args.data_dir);
    tracing::info!(count = watchers.len(), "loaded watchers");

    let state = Arc::new(Mutex::new(WatchState {
        switches: HashMap::new(),
        watchers,
        agentd_socket: args.agentd_socket.clone(),
        data_dir: args.data_dir,
    }));

    let cancel = CancellationToken::new();

    // Spawn background watcher loop
    let watcher_state = state.clone();
    let check_interval = args.check_interval;
    let watcher_cancel = cancel.clone();
    tokio::spawn(async move {
        watcher_loop(watcher_state, check_interval, watcher_cancel).await;
    });

    // Spawn background switch probation checker
    let switch_state = state.clone();
    let switch_cancel = cancel.clone();
    tokio::spawn(async move {
        switch_probation_loop(switch_state, switch_cancel).await;
    });

    let app = Router::new()
        // SafeSwitch
        .route("/switch/begin", post(api::switch_begin_handler))
        .route("/switch/status/{id}", get(api::switch_status_handler))
        .route("/switch/commit/{id}", post(api::switch_commit_handler))
        .route("/switch/rollback/{id}", post(api::switch_rollback_handler))
        // Watchers
        .route("/watcher/add", post(api::watcher_add_handler))
        .route("/watcher/list", get(api::watcher_list_handler))
        .route("/watcher/remove/{id}", delete(api::watcher_remove_handler))
        // Health
        .route("/health", get(api::health_handler))
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1 MiB
        .with_state(state);

    // Remove existing socket
    if Path::new(&args.socket).exists() {
        std::fs::remove_file(&args.socket).expect("failed to remove existing socket");
    }

    if let Some(parent) = Path::new(&args.socket).parent() {
        std::fs::create_dir_all(parent).expect("failed to create socket parent directory");
    }

    let listener = UnixListener::bind(&args.socket).expect("failed to bind Unix socket");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&args.socket, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(error = %e, "failed to set socket permissions");
        }
    }

    tracing::info!(socket = %args.socket, "osmoda-watch listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    // Signal background loops to stop
    cancel.cancel();
    tracing::info!("osmoda-watch shutdown complete");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

/// Background loop that runs watcher health checks on interval.
async fn watcher_loop(state: Arc<Mutex<WatchState>>, check_interval: u64, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(check_interval));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("watcher loop shutting down");
                break;
            }
            _ = interval.tick() => {
                let mut st = state.lock().await;
                let agentd_socket = st.agentd_socket.clone();
                for watcher in &mut st.watchers {
                    let actions = watcher::run_watcher_cycle(watcher).await;
                    for action in &actions {
                        tracing::info!(watcher = %watcher.name, action = %action, "watcher action");
                    }
                    // Log escalation events to agentd (best-effort)
                    if !actions.is_empty() {
                        let watcher_name = watcher.name.clone();
                        let payload = serde_json::json!({
                            "watcher": watcher_name,
                            "actions": actions.iter().map(|a| a.to_string()).collect::<Vec<_>>(),
                        }).to_string();
                        let sock = agentd_socket.clone();
                        tokio::spawn(async move {
                            if let Err(e) = agentd_post_event(&sock, "watch.watcher.escalation", &payload).await {
                                tracing::debug!(error = %e, "failed to log watcher escalation to agentd (non-fatal)");
                            }
                        });
                    }
                }
            }
        }
    }
}

/// Background loop that checks switch sessions for expiry and auto-commits/rollbacks.
async fn switch_probation_loop(state: Arc<Mutex<WatchState>>, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("switch probation loop shutting down");
                break;
            }
            _ = interval.tick() => {
                let mut st = state.lock().await;
                let agentd_socket = st.agentd_socket.clone();
                let active_ids: Vec<String> = st
                    .switches
                    .values()
                    .filter(|s| s.is_active())
                    .map(|s| s.id.clone())
                    .collect();

                for id in active_ids {
                    let session = match st.switches.get(&id) {
                        Some(s) => s.clone(),
                        None => continue,
                    };

                    if !session.is_expired() {
                        // Run health checks
                        let (passed, failures) =
                            switch::run_health_checks(&session.health_checks).await;

                        if !passed {
                            // Health check failed — rollback
                            tracing::warn!(
                                switch_id = %id,
                                failures = ?failures,
                                "health check failed, rolling back"
                            );

                            // Log rollback.triggered to agentd (best-effort)
                            let payload = serde_json::json!({
                                "switch_id": id,
                                "reason": "health_check_failure",
                                "failures": failures,
                            }).to_string();
                            let sock = agentd_socket.clone();
                            let id_for_log = id.clone();
                            tokio::spawn(async move {
                                if let Err(e) = agentd_post_event(&sock, "watch.rollback.triggered", &payload).await {
                                    tracing::debug!(error = %e, switch_id = %id_for_log, "failed to log rollback.triggered (non-fatal)");
                                }
                            });

                            let rollback_result = switch::rollback_generation().await;
                            let rollback_ok = rollback_result.is_ok();
                            if let Err(e) = rollback_result {
                                tracing::error!(error = %e, "auto-rollback failed");
                            }

                            // Log rollback.result to agentd (best-effort)
                            let result_payload = serde_json::json!({
                                "switch_id": id,
                                "success": rollback_ok,
                                "generation": session.previous_generation,
                            }).to_string();
                            let sock2 = agentd_socket.clone();
                            let id_for_log2 = id.clone();
                            tokio::spawn(async move {
                                if let Err(e) = agentd_post_event(&sock2, "watch.rollback.result", &result_payload).await {
                                    tracing::debug!(error = %e, switch_id = %id_for_log2, "failed to log rollback.result (non-fatal)");
                                }
                            });

                            if let Some(s) = st.switches.get_mut(&id) {
                                s.status = SwitchStatus::RolledBack {
                                    reason: format!("health check failures: {}", failures.join("; ")),
                                    rolled_back_at: chrono::Utc::now().to_rfc3339(),
                                };
                            }
                        }
                        continue;
                    }

                    // TTL expired — run final health check
                    let (passed, failures) =
                        switch::run_health_checks(&session.health_checks).await;

                    if passed {
                        // Auto-commit
                        tracing::info!(switch_id = %id, "TTL expired, all checks pass — auto-commit");
                        if let Some(s) = st.switches.get_mut(&id) {
                            s.status = SwitchStatus::Committed {
                                committed_at: chrono::Utc::now().to_rfc3339(),
                            };
                        }
                    } else {
                        // Auto-rollback on TTL expiry
                        tracing::warn!(switch_id = %id, "TTL expired, checks failed — auto-rollback");

                        // Log rollback.triggered to agentd (best-effort)
                        let payload = serde_json::json!({
                            "switch_id": id,
                            "reason": "ttl_expired_with_failures",
                            "failures": failures,
                        }).to_string();
                        let sock = agentd_socket.clone();
                        let id_for_log = id.clone();
                        tokio::spawn(async move {
                            if let Err(e) = agentd_post_event(&sock, "watch.rollback.triggered", &payload).await {
                                tracing::debug!(error = %e, switch_id = %id_for_log, "failed to log rollback.triggered (non-fatal)");
                            }
                        });

                        let rollback_result = switch::rollback_generation().await;
                        let rollback_ok = rollback_result.is_ok();
                        if let Err(e) = rollback_result {
                            tracing::error!(error = %e, "auto-rollback failed");
                        }

                        // Log rollback.result to agentd (best-effort)
                        let result_payload = serde_json::json!({
                            "switch_id": id,
                            "success": rollback_ok,
                            "generation": session.previous_generation,
                        }).to_string();
                        let sock2 = agentd_socket.clone();
                        let id_for_log2 = id.clone();
                        tokio::spawn(async move {
                            if let Err(e) = agentd_post_event(&sock2, "watch.rollback.result", &result_payload).await {
                                tracing::debug!(error = %e, switch_id = %id_for_log2, "failed to log rollback.result (non-fatal)");
                            }
                        });

                        if let Some(s) = st.switches.get_mut(&id) {
                            s.status = SwitchStatus::RolledBack {
                                reason: format!("TTL expired with failures: {}", failures.join("; ")),
                                rolled_back_at: chrono::Utc::now().to_rfc3339(),
                            };
                        }
                    }
                }
            }
        }
    }
}
