mod api;
mod ledger;
mod state;

use std::path::Path;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use clap::Parser;
use tokio::net::UnixListener;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use crate::state::{AppState, SharedState};

/// AgentOS daemon — the core system daemon providing ledger, system queries, and memory APIs.
#[derive(Parser, Debug)]
#[command(name = "agentd", version, about)]
struct Args {
    /// Path to the Unix domain socket to listen on.
    #[arg(long, default_value = "/run/agentos/agentd.sock")]
    socket: String,

    /// Directory for persistent state (SQLite ledger, etc.).
    #[arg(long, default_value = "/var/lib/agentos")]
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
        .with_state(shared_state);

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
        .await
        .expect("server error");
}
