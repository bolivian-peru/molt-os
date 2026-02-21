mod api;
mod ledger;
mod state;

use std::path::Path;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use clap::Parser;
use tokio::net::UnixListener;
use tokio::signal;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use crate::state::{AppState, SharedState};

/// osModa system daemon — the core daemon providing ledger, system queries, and memory APIs.
#[derive(Parser, Debug)]
#[command(name = "agentd", version, about)]
struct Args {
    /// Path to the Unix domain socket to listen on.
    #[arg(long, default_value = "/run/osmoda/agentd.sock")]
    socket: String,

    /// Directory for persistent state (SQLite ledger, etc.).
    #[arg(long, default_value = "/var/lib/osmoda")]
    state_dir: String,
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    tracing::info!(socket = %args.socket, state_dir = %args.state_dir, "starting agentd");

    // Ensure state directory exists
    std::fs::create_dir_all(&args.state_dir).expect("failed to create state directory");

    // Initialize SQLite ledger
    let ledger_path = Path::new(&args.state_dir).join("ledger.db");
    let ledger = ledger::Ledger::new(
        ledger_path
            .to_str()
            .expect("invalid ledger path"),
    )
    .expect("failed to initialize ledger");

    // Verify chain integrity on startup
    match ledger.verify() {
        Ok(true) => tracing::info!("ledger chain integrity verified"),
        Ok(false) => tracing::warn!("ledger chain integrity check FAILED — chain may be tampered"),
        Err(e) => tracing::error!(error = %e, "failed to verify ledger chain"),
    }

    // Log daemon startup event
    if let Err(e) = ledger.append("daemon.start", "agentd", &format!("{{\"socket\":\"{}\",\"state_dir\":\"{}\"}}", args.socket, args.state_dir)) {
        tracing::error!(error = %e, "failed to log daemon start event");
    }

    // Build shared state
    let sys = sysinfo::System::new_all();
    let shared_state: SharedState = Arc::new(AppState {
        ledger: Mutex::new(ledger),
        sys: Mutex::new(sys),
        state_dir: args.state_dir.clone(),
    });

    // Build the axum router
    let app = Router::new()
        .route("/health", get(api::health::health_handler))
        .route("/system/query", post(api::system::system_query_handler))
        .route("/events/log", get(api::events::events_log_handler))
        .route("/memory/ingest", post(api::memory::memory_ingest_handler))
        .route("/memory/recall", post(api::memory::memory_recall_handler))
        .route("/memory/store", post(api::memory::memory_store_handler))
        .route("/memory/health", get(api::memory::memory_health_handler))
        // Agent Card (EIP-8004)
        .route("/agent/card", get(api::agent_card::agent_card_handler))
        .route("/agent/card/generate", post(api::agent_card::agent_card_generate_handler))
        // Receipts + Incidents
        .route("/receipts", get(api::receipts::receipts_handler))
        .route("/incident/create", post(api::receipts::incident_create_handler))
        .route("/incident/{id}/step", post(api::receipts::incident_step_handler))
        .route("/incident/{id}", get(api::receipts::incident_get_handler))
        .route("/incidents", get(api::receipts::incidents_list_handler))
        // Backup
        .route("/backup/create", post(api::backup::backup_create_handler))
        .route("/backup/list", get(api::backup::backup_list_handler))
        .route("/backup/restore", post(api::backup::backup_restore_handler))
        .with_state(shared_state.clone());

    // Remove existing socket file if present
    if Path::new(&args.socket).exists() {
        std::fs::remove_file(&args.socket).expect("failed to remove existing socket file");
    }

    // Ensure socket parent directory exists
    if let Some(parent) = Path::new(&args.socket).parent() {
        std::fs::create_dir_all(parent).expect("failed to create socket parent directory");
    }

    // Bind to Unix socket
    let listener = UnixListener::bind(&args.socket).expect("failed to bind Unix socket");

    tracing::info!(socket = %args.socket, "agentd listening");

    // Serve
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    // Flush WAL on shutdown for data integrity
    {
        let ledger = shared_state.ledger.lock().await;
        if let Err(e) = ledger.flush() {
            tracing::warn!(error = %e, "WAL flush failed during shutdown");
        }
    }

    tracing::info!("agentd shutdown complete");
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
