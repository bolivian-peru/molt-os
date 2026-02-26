mod api;
mod routine;
mod scheduler;

use std::path::Path;
use std::sync::Arc;

use axum::routing::{delete, get, post};
use axum::Router;
use clap::Parser;
use tokio::net::UnixListener;
use tokio::signal;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::routine::Routine;

/// osModa Routines Engine — background cron/event/webhook automation.
#[derive(Parser, Debug)]
#[command(name = "osmoda-routines", version, about)]
struct Args {
    /// Path to the Unix domain socket to listen on.
    #[arg(long, default_value = "/run/osmoda/routines.sock")]
    socket: String,

    /// Path to the agentd Unix socket.
    #[arg(long, default_value = "/run/osmoda/agentd.sock")]
    agentd_socket: String,

    /// Directory for persisted routine definitions.
    #[arg(long, default_value = "/var/lib/osmoda/routines")]
    routines_dir: String,
}

pub struct RoutinesState {
    pub routines: Vec<Routine>,
    pub agentd_socket: String,
    pub routines_dir: String,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    tracing::info!(socket = %args.socket, routines_dir = %args.routines_dir, "starting osmoda-routines");

    std::fs::create_dir_all(&args.routines_dir).expect("failed to create routines directory");

    let routines = api::load_routines(&args.routines_dir);
    tracing::info!(count = routines.len(), "loaded routines");

    let state = Arc::new(Mutex::new(RoutinesState {
        routines,
        agentd_socket: args.agentd_socket,
        routines_dir: args.routines_dir,
    }));

    let cancel = CancellationToken::new();

    // Spawn scheduler loop
    let scheduler_state = state.clone();
    let scheduler_cancel = cancel.clone();
    tokio::spawn(async move {
        scheduler_loop(scheduler_state, scheduler_cancel).await;
    });

    let app = Router::new()
        .route("/routine/add", post(api::routine_add_handler))
        .route("/routine/list", get(api::routine_list_handler))
        .route("/routine/remove/{id}", delete(api::routine_remove_handler))
        .route("/routine/trigger/{id}", post(api::routine_trigger_handler))
        .route("/routine/history", get(api::routine_history_handler))
        .route("/health", get(api::health_handler))
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

    tracing::info!(socket = %args.socket, "osmoda-routines listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    // Signal background loops to stop
    cancel.cancel();
    tracing::info!("osmoda-routines shutdown complete");
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

/// Background scheduler that ticks every 60 seconds and runs due routines.
async fn scheduler_loop(state: Arc<Mutex<RoutinesState>>, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("scheduler loop shutting down");
                break;
            }
            _ = interval.tick() => {
                let now = chrono::Utc::now();

                let st = state.lock().await;

                // Collect indices of routines that should run
                let due: Vec<usize> = st
                    .routines
                    .iter()
                    .enumerate()
                    .filter(|(_, r)| r.should_run(&now))
                    .map(|(i, _)| i)
                    .collect();

                // Execute due routines (clone action + socket to release lock)
                let agentd_socket = st.agentd_socket.clone();
                let actions: Vec<(usize, String, routine::RoutineAction)> = due
                    .iter()
                    .filter_map(|&i| {
                        st.routines
                            .get(i)
                            .map(|r| (i, r.name.clone(), r.action.clone()))
                    })
                    .collect();

                // Release lock for execution
                drop(st);

                for (idx, name, action) in actions {
                    let result = routine::execute_action(&action, &agentd_socket).await;
                    match &result {
                        Ok(output) => {
                            tracing::debug!(routine = %name, output_len = output.len(), "routine completed");
                        }
                        Err(err) => {
                            tracing::warn!(routine = %name, error = %err, "routine failed");
                        }
                    }

                    // Update last_run and run_count
                    let mut st = state.lock().await;
                    if let Some(r) = st.routines.get_mut(idx) {
                        r.last_run = Some(now.to_rfc3339());
                        r.run_count += 1;
                    }
                }
            }
        }
    }
}
