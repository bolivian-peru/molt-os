mod api;
mod policy;
mod receipt;
mod signer;
mod tx_eth;
mod tx_sol;

use std::path::Path;
use std::sync::Arc;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;
use clap::Parser;
use tokio::net::UnixListener;
use tokio::signal;
use tokio::sync::Mutex;
use tracing_subscriber::EnvFilter;

use crate::policy::PolicyEngine;
use crate::receipt::ReceiptLogger;
use crate::signer::LocalKeyBackend;

/// osModa Key Daemon — OS-native crypto wallet management with policy-gated signing.
/// Runs with zero network access (PrivateNetwork=true). Communicates only via Unix socket.
#[derive(Parser, Debug)]
#[command(name = "osmoda-keyd", version, about)]
struct Args {
    /// Path to the Unix domain socket to listen on.
    #[arg(long, default_value = "/run/osmoda/keyd.sock")]
    socket: String,

    /// Directory for wallet keys and metadata.
    #[arg(long, default_value = "/var/lib/osmoda/keyd")]
    data_dir: String,

    /// Path to the policy rules JSON file.
    #[arg(long, default_value = "/var/lib/osmoda/keyd/policy.json")]
    policy_file: String,

    /// Path to the agentd Unix socket (for receipt logging).
    #[arg(long, default_value = "/run/osmoda/agentd.sock")]
    agentd_socket: String,
}

pub struct KeydState {
    pub signer: Mutex<LocalKeyBackend>,
    pub policy: Mutex<PolicyEngine>,
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

    tracing::info!(socket = %args.socket, data_dir = %args.data_dir, "starting osmoda-keyd");

    // Initialize key backend
    std::fs::create_dir_all(&args.data_dir).expect("failed to create data directory");

    // Harden permissions on startup — fix any files left at 644 by older versions
    #[cfg(unix)]
    harden_permissions(Path::new(&args.data_dir));

    let signer =
        LocalKeyBackend::new(Path::new(&args.data_dir)).expect("failed to initialize key backend");

    let policy = PolicyEngine::new(Path::new(&args.policy_file))
        .expect("failed to initialize policy engine");

    let receipt_logger = ReceiptLogger::new(&args.agentd_socket);

    let state = Arc::new(KeydState {
        signer: Mutex::new(signer),
        policy: Mutex::new(policy),
        receipt_logger,
    });

    let app = Router::new()
        .route("/wallet/create", post(api::wallet_create_handler))
        .route("/wallet/list", get(api::wallet_list_handler))
        .route("/wallet/sign", post(api::wallet_sign_handler))
        .route("/wallet/send", post(api::wallet_send_handler))
        .route("/wallet/delete", post(api::wallet_delete_handler))
        .route("/wallet/build_tx", post(api::wallet_build_tx_handler))
        .route("/health", get(api::health_handler))
        .layer(DefaultBodyLimit::max(1024 * 1024)) // 1 MiB
        .with_state(state);

    // Remove existing socket file if present
    if Path::new(&args.socket).exists() {
        std::fs::remove_file(&args.socket).expect("failed to remove existing socket");
    }

    if let Some(parent) = Path::new(&args.socket).parent() {
        std::fs::create_dir_all(parent).expect("failed to create socket parent directory");
    }

    let listener = UnixListener::bind(&args.socket).expect("failed to bind Unix socket");

    // Restrict socket permissions — only owner (root) can connect
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(&args.socket, std::fs::Permissions::from_mode(0o600)) {
            tracing::warn!(error = %e, "failed to set socket permissions");
        }
    }

    tracing::info!(socket = %args.socket, "osmoda-keyd listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    // State will be dropped, triggering LocalKeyBackend::drop() which zeroizes keys
    tracing::info!("osmoda-keyd shutting down, zeroizing cached keys");
    tracing::info!("osmoda-keyd shutdown complete");
}

/// Fix permissions on all keyd data files. Ensures .enc keys, wallets.json,
/// policy.json, master.key, and master.salt are all 0600. Directories get 0700.
#[cfg(unix)]
fn harden_permissions(data_dir: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let set_mode = |path: &Path, mode: u32| {
        if path.exists() {
            if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)) {
                tracing::warn!(path = %path.display(), error = %e, "failed to set permissions");
            }
        }
    };

    // Directories: 0700
    set_mode(data_dir, 0o700);
    set_mode(&data_dir.join("keys"), 0o700);

    // Sensitive files: 0600
    for name in &["master.key", "master.salt", "wallets.json", "policy.json"] {
        set_mode(&data_dir.join(name), 0o600);
    }

    // All encrypted key files: 0600
    if let Ok(entries) = std::fs::read_dir(data_dir.join("keys")) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|ext| ext == "enc") {
                set_mode(&entry.path(), 0o600);
            }
        }
    }

    tracing::info!("file permissions hardened");
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
