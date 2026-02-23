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
use serde::{Deserialize, Serialize};
use tokio::net::{TcpListener, TcpStream, UnixListener};
use tokio::signal;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

use crate::identity::MeshIdentity;
use crate::messages::MeshMessage;
use crate::peers::{ConnectionState, PeerInfo};
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

/// A message in a group room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMessage {
    pub from: String,
    pub text: String,
    pub ts: String,
    pub seq: u64,
}

/// A group chat room.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
    pub messages: Vec<RoomMessage>,
    pub created_at: String,
}

pub struct MeshState {
    pub identity: MeshIdentity,
    pub peers: Vec<PeerInfo>,
    pub connections: HashMap<String, Arc<MeshConnection>>,
    pub data_dir: String,
    pub listen_endpoint: String,
    pub receipt_logger: ReceiptLogger,
    pub rooms: Vec<Room>,
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
        rooms: Vec::new(),
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
        // Rooms
        .route("/rooms", get(api::rooms_list_handler))
        .route("/room/create", post(api::room_create_handler))
        .route("/room/join", post(api::room_join_handler))
        .route("/room/send", post(api::room_send_handler))
        .route("/room/history", get(api::room_history_handler))
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

/// Post a JSON body to agentd /memory/ingest over Unix socket. Best-effort.
pub async fn post_to_agentd(socket_path: &str, body: String) -> anyhow::Result<()> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::Request;
    use hyper_util::rt::TokioIo;
    use tokio::net::UnixStream;

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
        .body(Full::new(Bytes::from(body)))?;
    let _resp = sender.send_request(req).await?;
    Ok(())
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
                            let (keys, agentd_socket) = {
                                let st = state.lock().await;
                                let keys = HandshakeKeys {
                                    ed25519_bytes: st.identity.ed25519_signing_key.to_bytes(),
                                    noise_private: st.identity.noise_static_keypair.private.clone(),
                                    noise_public: st.identity.noise_static_keypair.public.clone(),
                                    mlkem_dk_bytes: st.identity.mlkem_dk_bytes.clone(),
                                    mlkem_ek_bytes: st.identity.mlkem_ek_bytes.clone(),
                                    public_identity: st.identity.public_identity.clone(),
                                };
                                let sock = st.receipt_logger.agentd_socket.clone();
                                (keys, sock)
                            };
                            let temp_identity = keys.into_identity();

                            match handshake::respond_handshake(&mut stream, &temp_identity).await {
                                Ok(result) => {
                                    let peer_id = result.peer_identity.instance_id.clone();
                                    tracing::info!(peer_id = %peer_id, "peer handshake completed");

                                    let connection = Arc::new(MeshConnection::new(
                                        peer_id.clone(),
                                        stream,
                                        result.transport,
                                        result.pq_rekey_material,
                                    ));

                                    let peer_noise_pubkey = result.peer_identity.noise_static_pubkey.clone();
                                    let peer_mlkem_key = result.peer_identity.mlkem_encap_key.clone();
                                    let conn_clone = connection.clone();
                                    let logger = {
                                        let mut st = state.lock().await;
                                        st.connections.insert(peer_id.clone(), connection);

                                        // Update existing peer or add as new inbound peer
                                        if let Some(peer) = st.peers.iter_mut().find(|p| p.id == peer_id) {
                                            peer.connection_state = ConnectionState::Connected {
                                                since: chrono::Utc::now().to_rfc3339(),
                                            };
                                            peer.last_seen = Some(chrono::Utc::now().to_rfc3339());
                                        } else {
                                            // Unknown inbound peer — add to peers list with empty endpoint
                                            let now = chrono::Utc::now().to_rfc3339();
                                            st.peers.push(PeerInfo {
                                                id: peer_id.clone(),
                                                label: format!("inbound-{}", &peer_id[..8.min(peer_id.len())]),
                                                noise_static_pubkey: peer_noise_pubkey,
                                                mlkem_encap_key: peer_mlkem_key,
                                                endpoint: String::new(), // not known from inbound
                                                added_at: now.clone(),
                                                last_seen: Some(now.clone()),
                                                connection_state: ConnectionState::Connected { since: now },
                                            });
                                            if let Err(e) = peers::save_peers(&st.peers, &st.data_dir) {
                                                tracing::warn!(error = %e, "failed to save inbound peer");
                                            }
                                        }

                                        st.receipt_logger.clone()
                                    }; // lock released — safe to await now

                                    let peer_id_for_log = peer_id.clone();
                                    tokio::spawn(async move { logger.log_connect(&peer_id_for_log).await; });

                                    // Spawn recv loop after releasing the lock
                                    spawn_recv_loop(peer_id, conn_clone, state, agentd_socket);
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

/// Spawn a recv loop task for an established connection.
fn spawn_recv_loop(
    peer_id: String,
    connection: Arc<MeshConnection>,
    state: Arc<Mutex<MeshState>>,
    agentd_socket: String,
) {
    tokio::spawn(async move {
        recv_loop(peer_id, connection, state, agentd_socket).await;
    });
}

/// Read messages from a connection and dispatch them.
async fn recv_loop(
    peer_id: String,
    connection: Arc<MeshConnection>,
    state: Arc<Mutex<MeshState>>,
    agentd_socket: String,
) {
    tracing::debug!(peer_id = %peer_id, "recv loop started");
    loop {
        match connection.recv_message().await {
            Ok(msg) => {
                dispatch_incoming(peer_id.clone(), msg, state.clone(), agentd_socket.clone()).await;
            }
            Err(e) => {
                tracing::warn!(peer_id = %peer_id, error = %e, "recv error, disconnecting peer");
                let logger = {
                    let mut st = state.lock().await;
                    st.connections.remove(&peer_id);
                    if let Some(peer) = st.peers.iter_mut().find(|p| p.id == peer_id) {
                        peer.connection_state = ConnectionState::Disconnected;
                    }
                    if let Err(e) = peers::save_peers(&st.peers, &st.data_dir) {
                        tracing::warn!(error = %e, "failed to save peers after recv disconnect");
                    }
                    st.receipt_logger.clone()
                }; // lock released — safe to await now
                let peer_id_for_log = peer_id.clone();
                tokio::spawn(async move { logger.log_disconnect(&peer_id_for_log).await; });
                break;
            }
        }
    }
    tracing::debug!(peer_id = %peer_id, "recv loop ended");
}

/// Dispatch a received message to the appropriate handler.
async fn dispatch_incoming(
    peer_id: String,
    msg: MeshMessage,
    state: Arc<Mutex<MeshState>>,
    agentd_socket: String,
) {
    match msg {
        MeshMessage::Heartbeat { .. } => {
            // Update last_seen timestamp
            let mut st = state.lock().await;
            if let Some(peer) = st.peers.iter_mut().find(|p| p.id == peer_id) {
                peer.last_seen = Some(chrono::Utc::now().to_rfc3339());
            }
        }
        MeshMessage::HealthReport { hostname, cpu, memory, uptime } => {
            let body = serde_json::json!({
                "source": "osmoda-mesh",
                "content": serde_json::json!({
                    "peer_id": peer_id,
                    "hostname": hostname,
                    "cpu": cpu,
                    "memory": memory,
                    "uptime": uptime,
                }).to_string(),
                "category": "mesh.health_report",
                "tags": ["mesh", "health_report", peer_id],
            }).to_string();
            let sock = agentd_socket.clone();
            tokio::spawn(async move {
                if let Err(e) = post_to_agentd(&sock, body).await {
                    tracing::debug!(error = %e, "failed to log mesh.health_report (non-fatal)");
                }
            });
        }
        MeshMessage::Alert { severity, title, detail } => {
            let severity_str = format!("{severity:?}").to_lowercase();
            if matches!(severity, crate::messages::AlertSeverity::Critical) {
                tracing::warn!(peer_id = %peer_id, title = %title, detail = %detail, "critical alert from peer");
            }
            let body = serde_json::json!({
                "source": "osmoda-mesh",
                "content": serde_json::json!({
                    "peer_id": peer_id,
                    "severity": severity_str,
                    "title": title,
                    "detail": detail,
                }).to_string(),
                "category": "mesh.alert",
                "tags": ["mesh", "alert", peer_id],
            }).to_string();
            let sock = agentd_socket.clone();
            tokio::spawn(async move {
                if let Err(e) = post_to_agentd(&sock, body).await {
                    tracing::debug!(error = %e, "failed to log mesh.alert (non-fatal)");
                }
            });
        }
        MeshMessage::Chat { from, text, room_id: None } => {
            tracing::debug!(peer_id = %peer_id, from = %from, "DM received");
            let body = serde_json::json!({
                "source": "osmoda-mesh",
                "content": serde_json::json!({
                    "peer_id": peer_id,
                    "from": from,
                    "text": text,
                }).to_string(),
                "category": "mesh.dm",
                "tags": ["mesh", "dm", from],
            }).to_string();
            let sock = agentd_socket.clone();
            tokio::spawn(async move {
                if let Err(e) = post_to_agentd(&sock, body).await {
                    tracing::debug!(error = %e, "failed to log mesh.dm (non-fatal)");
                }
            });
        }
        MeshMessage::Chat { from, text, room_id: Some(rid) } => {
            handle_room_message(rid, from, text, state, agentd_socket).await;
        }
        MeshMessage::PqExchange { .. } => {
            tracing::warn!(peer_id = %peer_id, "unexpected PqExchange post-handshake, ignoring");
        }
    }
}

/// Handle an incoming group room message.
async fn handle_room_message(
    room_id: String,
    from: String,
    text: String,
    state: Arc<Mutex<MeshState>>,
    agentd_socket: String,
) {
    let mut st = state.lock().await;
    if let Some(room) = st.rooms.iter_mut().find(|r| r.id == room_id) {
        let seq = room.messages.len() as u64;
        room.messages.push(RoomMessage {
            from: from.clone(),
            text: text.clone(),
            ts: chrono::Utc::now().to_rfc3339(),
            seq,
        });
        tracing::debug!(room_id = %room_id, from = %from, "room message appended");
    } else {
        tracing::warn!(room_id = %room_id, "received message for unknown room, ignoring");
        return;
    }
    drop(st);

    // Log to agentd (best-effort)
    let body = serde_json::json!({
        "source": "osmoda-mesh",
        "content": serde_json::json!({
            "room_id": room_id,
            "from": from,
            "text": text,
        }).to_string(),
        "category": "mesh.room.message",
        "tags": ["mesh", "room", room_id, from],
    }).to_string();
    tokio::spawn(async move {
        if let Err(e) = post_to_agentd(&agentd_socket, body).await {
            tracing::debug!(error = %e, "failed to log mesh.room.message (non-fatal)");
        }
    });
}

/// Initiate an outbound connection to a peer. Retries up to 3 times with backoff.
pub async fn initiate_outbound_connection(
    state: Arc<Mutex<MeshState>>,
    peer_id: String,
    endpoint: String,
) {
    let delays = [
        tokio::time::Duration::from_secs(0),
        tokio::time::Duration::from_secs(5),
        tokio::time::Duration::from_secs(15),
    ];

    for (attempt, &delay) in delays.iter().enumerate() {
        if delay.as_secs() > 0 {
            tokio::time::sleep(delay).await;
        }

        tracing::info!(peer_id = %peer_id, endpoint = %endpoint, attempt = attempt + 1, "initiating outbound connection");

        // Extract keys without holding the lock during connection
        let (keys, agentd_socket) = {
            let st = state.lock().await;
            let keys = HandshakeKeys {
                ed25519_bytes: st.identity.ed25519_signing_key.to_bytes(),
                noise_private: st.identity.noise_static_keypair.private.clone(),
                noise_public: st.identity.noise_static_keypair.public.clone(),
                mlkem_dk_bytes: st.identity.mlkem_dk_bytes.clone(),
                mlkem_ek_bytes: st.identity.mlkem_ek_bytes.clone(),
                public_identity: st.identity.public_identity.clone(),
            };
            let sock = st.receipt_logger.agentd_socket.clone();
            (keys, sock)
        };

        // Attempt TCP connection with 10s timeout
        let connect_result = tokio::time::timeout(
            tokio::time::Duration::from_secs(10),
            TcpStream::connect(&endpoint),
        )
        .await;

        let mut stream = match connect_result {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                tracing::warn!(peer_id = %peer_id, endpoint = %endpoint, error = %e, "TCP connect failed");
                continue;
            }
            Err(_) => {
                tracing::warn!(peer_id = %peer_id, endpoint = %endpoint, "TCP connect timed out");
                continue;
            }
        };

        let temp_identity = keys.into_identity();
        match handshake::initiate_handshake(&mut stream, &temp_identity).await {
            Ok(result) => {
                let connected_peer_id = result.peer_identity.instance_id.clone();
                tracing::info!(peer_id = %connected_peer_id, "outbound handshake completed");

                let connection = Arc::new(MeshConnection::new(
                    connected_peer_id.clone(),
                    stream,
                    result.transport,
                    result.pq_rekey_material,
                ));

                let conn_clone = connection.clone();
                let logger = {
                    let mut st = state.lock().await;
                    st.connections.insert(connected_peer_id.clone(), connection);

                    if let Some(peer) = st.peers.iter_mut().find(|p| p.id == peer_id) {
                        peer.connection_state = ConnectionState::Connected {
                            since: chrono::Utc::now().to_rfc3339(),
                        };
                        peer.last_seen = Some(chrono::Utc::now().to_rfc3339());
                    }

                    st.receipt_logger.clone()
                }; // lock released — safe to await now

                let cpi = connected_peer_id.clone();
                tokio::spawn(async move { logger.log_connect(&cpi).await; });

                // Spawn recv loop after releasing the lock
                spawn_recv_loop(connected_peer_id, conn_clone, state.clone(), agentd_socket);
                return; // Success
            }
            Err(e) => {
                tracing::warn!(peer_id = %peer_id, error = %e, "outbound handshake failed");
                // Update peer state to show failure
                let mut st = state.lock().await;
                if let Some(peer) = st.peers.iter_mut().find(|p| p.id == peer_id) {
                    peer.connection_state = ConnectionState::Failed {
                        reason: format!("handshake failed: {e}"),
                        at: chrono::Utc::now().to_rfc3339(),
                    };
                }
            }
        }
    }

    // All attempts failed
    tracing::warn!(peer_id = %peer_id, endpoint = %endpoint, "all outbound connection attempts failed");
    let mut st = state.lock().await;
    if let Some(peer) = st.peers.iter_mut().find(|p| p.id == peer_id) {
        peer.connection_state = ConnectionState::Disconnected;
    }
}

/// Background loop that checks connection health and detects dead peers.
async fn connection_health_loop(state: Arc<Mutex<MeshState>>, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("connection health loop shutting down");
                break;
            }
            _ = interval.tick() => {
                let stale_threshold = chrono::Duration::seconds(90);
                let now = chrono::Utc::now();

                // Collect stale connections (have connection but last_seen > 90s ago or never)
                let stale_peers: Vec<(String, Arc<MeshConnection>)> = {
                    let st = state.lock().await;
                    st.connections.iter().filter_map(|(id, conn)| {
                        let peer = st.peers.iter().find(|p| p.id == *id)?;
                        let stale = match &peer.last_seen {
                            None => true,
                            Some(ts) => {
                                if let Ok(t) = chrono::DateTime::parse_from_rfc3339(ts) {
                                    now.signed_duration_since(t.with_timezone(&chrono::Utc)) > stale_threshold
                                } else {
                                    true
                                }
                            }
                        };
                        if stale { Some((id.clone(), conn.clone())) } else { None }
                    }).collect()
                };

                // For each stale peer, send a heartbeat probe
                for (peer_id, connection) in stale_peers {
                    let heartbeat = MeshMessage::Heartbeat {
                        timestamp: now.to_rfc3339(),
                    };
                    if let Err(e) = connection.send_message(&heartbeat).await {
                        tracing::warn!(peer_id = %peer_id, error = %e, "heartbeat probe failed, disconnecting");
                        let logger = {
                            let mut st = state.lock().await;
                            st.connections.remove(&peer_id);
                            if let Some(peer) = st.peers.iter_mut().find(|p| p.id == peer_id) {
                                peer.connection_state = ConnectionState::Disconnected;
                            }
                            if let Err(e) = peers::save_peers(&st.peers, &st.data_dir) {
                                tracing::warn!(error = %e, "failed to save peers after health disconnect");
                            }
                            st.receipt_logger.clone()
                        }; // lock released — safe to await now
                        let peer_id_for_log = peer_id.clone();
                        tokio::spawn(async move { logger.log_disconnect(&peer_id_for_log).await; });
                    }
                }

                // Reconnect to disconnected peers that have known endpoints
                let disconnected_peers: Vec<(String, String)> = {
                    let st = state.lock().await;
                    st.peers.iter().filter_map(|p| {
                        let is_disconnected = matches!(&p.connection_state, ConnectionState::Disconnected);
                        let not_connected = !st.connections.contains_key(&p.id);
                        let has_endpoint = !p.endpoint.is_empty();
                        if is_disconnected && not_connected && has_endpoint {
                            Some((p.id.clone(), p.endpoint.clone()))
                        } else {
                            None
                        }
                    }).collect()
                };

                for (peer_id, endpoint) in disconnected_peers {
                    let st_clone = state.clone();
                    tokio::spawn(async move {
                        initiate_outbound_connection(st_clone, peer_id, endpoint).await;
                    });
                }

                let (peer_count, connected_count) = {
                    let st = state.lock().await;
                    (st.peers.len(), st.connections.len())
                };
                tracing::debug!(peers = peer_count, connected = connected_count, "connection health check");
            }
        }
    }
}
