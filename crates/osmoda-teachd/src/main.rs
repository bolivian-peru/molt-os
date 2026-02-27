mod api;
mod knowledge;
mod learner;
mod observer;
mod optimizer;
mod receipt;
mod teacher;

use std::path::Path;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use clap::Parser;
use rusqlite::Connection;
use tokio::net::UnixListener;
use tokio::signal;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::receipt::ReceiptLogger;

/// osModa Teaching Daemon — system learning and self-optimization.
#[derive(Parser, Debug)]
#[command(name = "osmoda-teachd", version, about)]
struct Args {
    /// Path to the Unix domain socket to listen on.
    #[arg(long, default_value = "/run/osmoda/teachd.sock")]
    socket: String,

    /// State directory for teachd data (SQLite DB, etc.).
    #[arg(long, default_value = "/var/lib/osmoda/teachd")]
    state_dir: String,

    /// Path to the agentd Unix socket (for receipt logging).
    #[arg(long, default_value = "/run/osmoda/agentd.sock")]
    agentd_socket: String,

    /// Path to the watch daemon Unix socket (for SafeSwitch).
    #[arg(long, default_value = "/run/osmoda/watch.sock")]
    watch_socket: String,
}

pub struct TeachdState {
    pub db: Connection,
    pub state_dir: String,
    pub agentd_socket: String,
    pub watch_socket: String,
    pub receipt_logger: ReceiptLogger,
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

    tracing::info!(
        socket = %args.socket,
        state_dir = %args.state_dir,
        "starting osmoda-teachd"
    );

    std::fs::create_dir_all(&args.state_dir).expect("failed to create state directory");

    // Open SQLite database with WAL mode
    let db_path = format!("{}/teachd.db", args.state_dir);
    let db = Connection::open(&db_path).expect("failed to open SQLite database");
    db.pragma_update(None, "journal_mode", "wal")
        .expect("failed to set WAL mode");
    db.pragma_update(None, "busy_timeout", "5000")
        .expect("failed to set busy_timeout");

    knowledge::init_db(&db).expect("failed to initialize database schema");

    let receipt_logger = ReceiptLogger::new(&args.agentd_socket);

    let state = Arc::new(Mutex::new(TeachdState {
        db,
        state_dir: args.state_dir,
        agentd_socket: args.agentd_socket,
        watch_socket: args.watch_socket,
        receipt_logger,
    }));

    let cancel = CancellationToken::new();

    // Spawn OBSERVE loop
    let observer_state = state.clone();
    let observer_cancel = cancel.clone();
    tokio::spawn(async move {
        observer::observer_loop(observer_state, observer_cancel).await;
    });

    // Spawn LEARN loop
    let learner_state = state.clone();
    let learner_cancel = cancel.clone();
    tokio::spawn(async move {
        learner::learner_loop(learner_state, learner_cancel).await;
    });

    let app = Router::new()
        .route("/health", get(api::health_handler))
        .route("/observations", get(api::observations_handler))
        .route("/patterns", get(api::patterns_handler))
        .route("/knowledge", get(api::knowledge_list_handler))
        .route("/knowledge/{id}", get(api::knowledge_get_handler))
        .route("/knowledge/create", post(api::knowledge_create_handler))
        .route("/knowledge/{id}/update", post(api::knowledge_update_handler))
        .route("/teach", post(api::teach_handler))
        .route("/optimize/suggest", post(api::optimize_suggest_handler))
        .route("/optimize/approve/{id}", post(api::optimize_approve_handler))
        .route("/optimize/apply/{id}", post(api::optimize_apply_handler))
        .route("/optimizations", get(api::optimizations_list_handler))
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1 MiB
        .with_state(state.clone());

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
        if let Err(e) =
            std::fs::set_permissions(&args.socket, std::fs::Permissions::from_mode(0o600))
        {
            tracing::warn!(error = %e, "failed to set socket permissions");
        }
    }

    tracing::info!(socket = %args.socket, "osmoda-teachd listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    cancel.cancel();
    tracing::info!("osmoda-teachd shutdown complete");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
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
