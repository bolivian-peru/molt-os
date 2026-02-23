mod api;
mod receipt;
mod server;

use std::path::Path;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;
use clap::Parser;
use tokio::net::UnixListener;
use tokio::signal;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::receipt::ReceiptLogger;
use crate::server::{ManagedServer, ServerConfig, ServerStatus};

/// osModa MCP Server Manager — lifecycle management for MCP servers.
#[derive(Parser, Debug)]
#[command(name = "osmoda-mcpd", version, about)]
struct Args {
    /// Path to the Unix domain socket to listen on.
    #[arg(long, default_value = "/run/osmoda/mcpd.sock")]
    socket: String,

    /// Path to the MCP servers config JSON file (NixOS-generated).
    #[arg(long, default_value = "/var/lib/osmoda/mcp/mcp-servers.json")]
    config: String,

    /// State directory for mcpd.
    #[arg(long, default_value = "/var/lib/osmoda/mcp")]
    state_dir: String,

    /// Path to the agentd Unix socket (for receipt logging).
    #[arg(long, default_value = "/run/osmoda/agentd.sock")]
    agentd_socket: String,

    /// Egress proxy port (injected as HTTP_PROXY for servers with allowedDomains).
    #[arg(long, default_value_t = 19999)]
    egress_port: u16,

    /// Path to write the OpenClaw MCP config file.
    #[arg(long, default_value = "/var/lib/osmoda/mcp/openclaw-mcp.json")]
    output_config: String,
}

pub struct McpdState {
    pub servers: Vec<ManagedServer>,
    pub config_path: String,
    pub agentd_socket: String,
    pub egress_port: u16,
    pub state_dir: String,
    pub output_config_path: String,
    pub receipt_logger: ReceiptLogger,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    tracing::info!(
        socket = %args.socket,
        config = %args.config,
        state_dir = %args.state_dir,
        "starting osmoda-mcpd"
    );

    std::fs::create_dir_all(&args.state_dir).expect("failed to create state directory");

    let receipt_logger = ReceiptLogger::new(&args.agentd_socket);

    // Load server configs from NixOS-generated JSON
    let server_configs = load_server_configs(&args.config);
    tracing::info!(count = server_configs.len(), "loaded MCP server configs");

    // Create managed servers from configs
    let mut servers: Vec<ManagedServer> = server_configs
        .into_iter()
        .map(ManagedServer::from_config)
        .collect();

    // Start all servers
    for srv in &mut servers {
        if let Err(e) = server::start_server(srv, args.egress_port).await {
            tracing::error!(server = %srv.config.name, error = %e, "failed to start MCP server");
            receipt_logger
                .log_event("server.start_failed", &srv.config.name, &e.to_string())
                .await;
        } else {
            receipt_logger
                .log_event("server.start", &srv.config.name, "started successfully")
                .await;
        }
    }

    // Write OpenClaw MCP config
    if let Err(e) =
        server::write_openclaw_config(&servers, &args.output_config, args.egress_port)
    {
        tracing::error!(error = %e, "failed to write OpenClaw MCP config");
    }

    let state = Arc::new(Mutex::new(McpdState {
        servers,
        config_path: args.config,
        agentd_socket: args.agentd_socket,
        egress_port: args.egress_port,
        state_dir: args.state_dir,
        output_config_path: args.output_config,
        receipt_logger,
    }));

    let cancel = CancellationToken::new();

    // Spawn server manager loop (health check + restart crashed servers)
    let manager_state = state.clone();
    let manager_cancel = cancel.clone();
    tokio::spawn(async move {
        server_manager_loop(manager_state, manager_cancel).await;
    });

    let app = Router::new()
        .route("/health", get(api::health_handler))
        .route("/servers", get(api::servers_list_handler))
        .route("/server/{name}", get(api::server_detail_handler))
        .route("/server/{name}/start", post(api::server_start_handler))
        .route("/server/{name}/stop", post(api::server_stop_handler))
        .route("/server/{name}/restart", post(api::server_restart_handler))
        .route("/reload", post(api::reload_handler))
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
            std::fs::set_permissions(&args.socket, std::fs::Permissions::from_mode(0o660))
        {
            tracing::warn!(error = %e, "failed to set socket permissions");
        }
    }

    tracing::info!(socket = %args.socket, "osmoda-mcpd listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    // On shutdown: kill all child processes
    cancel.cancel();
    let mut st = state.lock().await;
    for srv in &mut st.servers {
        if let Err(e) = server::stop_server(srv).await {
            tracing::warn!(server = %srv.config.name, error = %e, "failed to stop server on shutdown");
        }
    }
    tracing::info!("osmoda-mcpd shutdown complete");
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

fn load_server_configs(path: &str) -> Vec<ServerConfig> {
    if !Path::new(path).exists() {
        tracing::warn!(path = %path, "MCP config file not found, starting with no servers");
        return Vec::new();
    }

    match std::fs::read_to_string(path) {
        Ok(data) => match serde_json::from_str::<Vec<ServerConfig>>(&data) {
            Ok(configs) => configs,
            Err(e) => {
                tracing::error!(error = %e, "failed to parse MCP config JSON");
                Vec::new()
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "failed to read MCP config file");
            Vec::new()
        }
    }
}

/// Background loop that checks server health and restarts crashed servers.
async fn server_manager_loop(state: Arc<Mutex<McpdState>>, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("server manager loop shutting down");
                break;
            }
            _ = interval.tick() => {
                let mut st = state.lock().await;
                let egress_port = st.egress_port;
                let receipt_logger = st.receipt_logger.clone();

                // Find crashed servers first
                let mut crashed: Vec<usize> = Vec::new();
                for (i, srv) in st.servers.iter_mut().enumerate() {
                    if matches!(srv.status, ServerStatus::Running) {
                        server::check_server(srv);
                        if matches!(srv.status, ServerStatus::Failed) {
                            crashed.push(i);
                        }
                    }
                }

                // Restart crashed servers
                for i in crashed {
                    let name = st.servers[i].config.name.clone();
                    tracing::warn!(server = %name, "MCP server crashed, restarting");
                    receipt_logger
                        .log_event("server.crash", &name, "process exited unexpectedly")
                        .await;

                    st.servers[i].status = ServerStatus::Restarting;

                    if let Err(e) = server::start_server(&mut st.servers[i], egress_port).await {
                        tracing::error!(server = %name, error = %e, "failed to restart MCP server");
                        st.servers[i].status = ServerStatus::Failed;
                        st.servers[i].last_error = Some(e.to_string());
                    } else {
                        st.servers[i].restart_count += 1;
                        receipt_logger
                            .log_event("server.restart", &name, "restarted after crash")
                            .await;
                    }
                }
            }
        }
    }
}
