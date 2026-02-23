mod api;
mod handshake;
pub mod identity;
mod invite;
mod messages;
mod peers;
mod receipt;
mod transport;

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use axum::routing::{delete, get, post};
use axum::Router;
use clap::Parser;
use tokio::net::{TcpListener, UnixListener};
use tokio::signal;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::identity::MeshIdentity;
use crate::peers::PeerInfo;
use crate::receipt::ReceiptLogger;
use crate::transport::MeshConnection;

/// osModa Mesh Daemon — P2P encrypted agent-to-agent communication.
#[derive(Parser, Debug)]
#[command(name = "osmoda-mesh", version, about)]
struct Args {
    /// Path to the Unix domain socket for local API.
    #[arg(long, default_value = "/run/osmoda/mesh.sock")]
    socket: String,

    /// Directory for mesh state (keys, peers, identity).
    #[arg(long, default_value = "/var/lib/osmoda/mesh")]
    data_dir: String,

    /// Path to the agentd Unix socket (for receipt logging).
    #[arg(long, default_value = "/run/osmoda/agentd.sock")]
    agentd_socket: String,

    /// TCP listen address for incoming peer connections.
    #[arg(long, default_value = "0.0.0.0")]
    listen_addr: String,

    /// TCP listen port for incoming peer connections.
    #[arg(long, default_value_t = 18800)]
    listen_port: u16,
}

pub struct MeshState {
    pub identity: MeshIdentity,
    pub peers: Vec<PeerInfo>,
    pub connections: HashMap<String, Arc<MeshConnection>>,
    pub data_dir: String,
    pub listen_endpoint: String,
    pub receipt_logger: ReceiptLogger,
    pub pending_connections: Vec<String>,
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
        data_dir = %args.data_dir,
        listen = %format!("{}:{}", args.listen_addr, args.listen_port),
        "starting osmoda-mesh"
    );

    std::fs::create_dir_all(&args.data_dir).expect("failed to create mesh data directory");

    // Load or generate identity
    let data_dir_path = std::path::Path::new(&args.data_dir);
    let identity =
        MeshIdentity::load_or_create(data_dir_path).expect("failed to initialize mesh identity");
    tracing::info!(
        instance_id = %identity.public_identity.instance_id,
        "mesh identity ready"
    );

    // Load known peers
    let peers_list = peers::load_peers(&args.data_dir);
    tracing::info!(count = peers_list.len(), "loaded known peers");

    let listen_endpoint = format!("{}:{}", args.listen_addr, args.listen_port);

    let state = Arc::new(Mutex::new(MeshState {
        identity,
        peers: peers_list,
        connections: HashMap::new(),
        data_dir: args.data_dir.clone(),
        listen_endpoint: listen_endpoint.clone(),
        receipt_logger: ReceiptLogger::new(&args.agentd_socket),
        pending_connections: Vec::new(),
    }));

    let cancel = CancellationToken::new();

    // Spawn TCP listener for incoming peer connections
    let tcp_state = state.clone();
    let tcp_cancel = cancel.clone();
    let tcp_addr = format!("{}:{}", args.listen_addr, args.listen_port);
    tokio::spawn(async move {
        tcp_accept_loop(tcp_state, &tcp_addr, tcp_cancel).await;
    });

    // Spawn connection health checker
    let health_state = state.clone();
    let health_cancel = cancel.clone();
    tokio::spawn(async move {
        connection_health_loop(health_state, health_cancel).await;
    });

    // Build local API router
    let app = Router::new()
        // Invites
        .route("/invite/create", post(api::invite_create_handler))
        .route("/invite/accept", post(api::invite_accept_handler))
        // Peers
        .route("/peers", get(api::peers_list_handler))
        .route("/peer/{id}", get(api::peer_get_handler))
        .route("/peer/{id}/send", post(api::peer_send_handler))
        .route("/peer/{id}", delete(api::peer_disconnect_handler))
        // Identity
        .route("/identity/rotate", post(api::identity_rotate_handler))
        .route("/identity", get(api::identity_get_handler))
        // Health
        .route("/health", get(api::health_handler))
        .with_state(state);

    // Set up Unix socket
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

    tracing::info!(socket = %args.socket, "osmoda-mesh listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");

    cancel.cancel();
    tracing::info!("osmoda-mesh shutdown complete");
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

/// Extract the crypto material needed for a handshake from the shared state.
/// This avoids holding the mutex lock during the handshake.
struct HandshakeKeys {
    ed25519_bytes: [u8; 32],
    noise_private: Vec<u8>,
    noise_public: Vec<u8>,
    mlkem_dk_bytes: Vec<u8>,
    mlkem_ek_bytes: Vec<u8>,
    public_identity: identity::MeshPublicIdentity,
}

impl HandshakeKeys {
    fn into_identity(self) -> MeshIdentity {
        MeshIdentity {
            ed25519_signing_key: ed25519_dalek::SigningKey::from_bytes(&self.ed25519_bytes),
            noise_static_keypair: snow::Keypair {
                private: self.noise_private,
                public: self.noise_public,
            },
            mlkem_dk_bytes: self.mlkem_dk_bytes,
            mlkem_ek_bytes: self.mlkem_ek_bytes,
            public_identity: self.public_identity,
        }
    }
}

/// Background loop that accepts incoming TCP connections from peers.
async fn tcp_accept_loop(
    state: Arc<Mutex<MeshState>>,
    addr: &str,
    cancel: CancellationToken,
) {
    let listener = match TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(error = %e, addr = %addr, "failed to bind TCP listener");
            return;
        }
    };

    tracing::info!(addr = %addr, "TCP peer listener started");

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("TCP accept loop shutting down");
                break;
            }
            result = listener.accept() => {
                match result {
                    Ok((mut stream, peer_addr)) => {
                        tracing::info!(peer = %peer_addr, "incoming peer connection");
                        let state = state.clone();
                        tokio::spawn(async move {
                            // Extract keys briefly while holding the lock
                            let keys = {
                                let st = state.lock().await;
                                HandshakeKeys {
                                    ed25519_bytes: st.identity.ed25519_signing_key.to_bytes(),
                                    noise_private: st.identity.noise_static_keypair.private.clone(),
                                    noise_public: st.identity.noise_static_keypair.public.clone(),
                                    mlkem_dk_bytes: st.identity.mlkem_dk_bytes.clone(),
                                    mlkem_ek_bytes: st.identity.mlkem_ek_bytes.clone(),
                                    public_identity: st.identity.public_identity.clone(),
                                }
                            };
                            let temp_identity = keys.into_identity();

                            match handshake::respond_handshake(&mut stream, &temp_identity).await {
                                Ok(result) => {
                                    let peer_id = result.peer_identity.instance_id.clone();
                                    tracing::info!(peer_id = %peer_id, "peer handshake completed");

                                    let connection = Arc::new(MeshConnection {
                                        peer_id: peer_id.clone(),
                                        transport: Arc::new(Mutex::new(result.transport)),
                                        stream: Arc::new(Mutex::new(stream)),
                                        phase: transport::TransportPhase::Connected,
                                        pq_rekey_material: result.pq_rekey_material,
                                    });

                                    let mut st = state.lock().await;
                                    st.connections.insert(peer_id.clone(), connection);

                                    // Update peer state
                                    if let Some(peer) = st.peers.iter_mut().find(|p| p.id == peer_id) {
                                        peer.connection_state = peers::ConnectionState::Connected {
                                            since: chrono::Utc::now().to_rfc3339(),
                                        };
                                        peer.last_seen = Some(chrono::Utc::now().to_rfc3339());
                                    }

                                    st.receipt_logger.log_connect(&peer_id).await;
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        peer = %peer_addr,
                                        error = %e,
                                        "peer handshake failed"
                                    );
                                }
                            }
                        });
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "TCP accept error");
                    }
                }
            }
        }
    }
}

/// Background loop that checks connection health (dead peer detection).
async fn connection_health_loop(state: Arc<Mutex<MeshState>>, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("connection health loop shutting down");
                break;
            }
            _ = interval.tick() => {
                let st = state.lock().await;
                let peer_count = st.peers.len();
                let connected_count = st.connections.len();
                tracing::debug!(
                    peers = peer_count,
                    connected = connected_count,
                    "connection health check"
                );
            }
        }
    }
}
