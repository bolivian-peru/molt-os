use anyhow::{Context, Result};
use clap::Parser;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use http_body_util::Full;
use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tracing::{error, info, warn};

#[derive(Parser)]
#[command(name = "agentos-egress", about = "AgentOS egress proxy — domain-filtered HTTP CONNECT proxy")]
struct Cli {
    /// Port to listen on (localhost only)
    #[arg(long, default_value = "19999")]
    port: u16,

    /// State directory for configuration
    #[arg(long, default_value = "/var/lib/agentos")]
    state_dir: String,

    /// Default allowed domains (comma-separated)
    #[arg(long, default_value = "cache.nixos.org,channels.nixos.org,github.com,api.anthropic.com")]
    default_allow: String,
}

struct ProxyState {
    allowed_domains: HashSet<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    let allowed: HashSet<String> = cli
        .default_allow
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    info!(
        "Starting egress proxy on 127.0.0.1:{} with {} allowed domains",
        cli.port,
        allowed.len()
    );
    for domain in &allowed {
        info!("  Allowed: {domain}");
    }

    let state = Arc::new(ProxyState {
        allowed_domains: allowed,
    });

    let addr = SocketAddr::from(([127, 0, 0, 1], cli.port));
    let listener = TcpListener::bind(addr)
        .await
        .with_context(|| format!("Failed to bind to {addr}"))?;

    info!("Egress proxy listening on {addr}");

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        let state = state.clone();

        tokio::spawn(async move {
            let io = TokioIo::new(stream);
            let state = state.clone();

            if let Err(e) = http1::Builder::new()
                .preserve_header_case(true)
                .title_case_headers(true)
                .serve_connection(
                    io,
                    service_fn(move |req| {
                        let state = state.clone();
                        async move { handle_request(req, state, peer_addr).await }
                    }),
                )
                .with_upgrades()
                .await
            {
                error!("Connection error from {peer_addr}: {e}");
            }
        });
    }
}

async fn handle_request(
    req: Request<hyper::body::Incoming>,
    state: Arc<ProxyState>,
    peer_addr: SocketAddr,
) -> Result<Response<Full<bytes::Bytes>>, hyper::Error> {
    if req.method() == Method::CONNECT {
        // CONNECT tunnel — extract host
        let host = req.uri().authority().map(|a| a.host().to_lowercase());

        match host {
            Some(ref domain) if state.allowed_domains.contains(domain.as_str()) => {
                info!("CONNECT allowed: {domain} (from {peer_addr})");

                let target_host = req.uri().authority().unwrap().to_string();

                // Spawn tunnel
                tokio::spawn(async move {
                    match hyper::upgrade::on(req).await {
                        Ok(upgraded) => {
                            if let Err(e) = tunnel(upgraded, &target_host).await {
                                error!("Tunnel to {target_host} failed: {e}");
                            }
                        }
                        Err(e) => error!("Upgrade failed: {e}"),
                    }
                });

                Ok(Response::new(Full::default()))
            }
            Some(domain) => {
                warn!("CONNECT denied: {domain} (from {peer_addr}) — not in allowlist");
                let mut resp = Response::new(Full::from(
                    format!("Domain '{domain}' not in egress allowlist"),
                ));
                *resp.status_mut() = StatusCode::FORBIDDEN;
                Ok(resp)
            }
            None => {
                warn!("CONNECT denied: no host in request (from {peer_addr})");
                let mut resp =
                    Response::new(Full::from("Missing host in CONNECT request"));
                *resp.status_mut() = StatusCode::BAD_REQUEST;
                Ok(resp)
            }
        }
    } else {
        // Non-CONNECT requests — reject, we only support CONNECT tunneling
        let mut resp = Response::new(Full::from(
            "Only CONNECT method is supported. This is an egress proxy.",
        ));
        *resp.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
        Ok(resp)
    }
}

async fn tunnel(
    upgraded: hyper::upgrade::Upgraded,
    target: &str,
) -> Result<()> {
    let mut server = TcpStream::connect(target)
        .await
        .with_context(|| format!("Failed to connect to upstream {target}"))?;

    let mut upgraded = TokioIo::new(upgraded);

    let (mut client_reader, mut client_writer) = tokio::io::split(&mut upgraded);
    let (mut server_reader, mut server_writer) = server.split();

    let client_to_server = tokio::io::copy(&mut client_reader, &mut server_writer);
    let server_to_client = tokio::io::copy(&mut server_reader, &mut client_writer);

    tokio::select! {
        result = client_to_server => {
            if let Err(e) = result {
                error!("Client→Server copy error for {target}: {e}");
            }
        }
        result = server_to_client => {
            if let Err(e) = result {
                error!("Server→Client copy error for {target}: {e}");
            }
        }
    }

    Ok(())
}
