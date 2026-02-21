use anyhow::Result;
use serde::Serialize;

/// Logs wallet operations to the agentd ledger via HTTP-over-Unix-socket.
pub struct ReceiptLogger {
    agentd_socket: String,
}

#[derive(Debug, Serialize)]
pub struct WalletReceipt {
    pub wallet_id: String,
    pub action: String,
    pub chain: String,
    pub to: Option<String>,
    pub amount: Option<String>,
    pub policy_decision: String,
    pub timestamp: String,
}

impl ReceiptLogger {
    pub fn new(agentd_socket: &str) -> Self {
        Self {
            agentd_socket: agentd_socket.to_string(),
        }
    }

    /// Log a wallet receipt to the agentd ledger. Best-effort — failures are logged but not fatal.
    pub async fn log_receipt(&self, receipt: &WalletReceipt) {
        let payload = match serde_json::to_string(receipt) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize receipt");
                return;
            }
        };

        let body = serde_json::json!({
            "source": "osmoda-keyd",
            "content": payload,
            "category": format!("wallet.{}", receipt.action),
            "tags": ["wallet", &receipt.chain, &receipt.action],
        });

        let body_str = match serde_json::to_string(&body) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize receipt body");
                return;
            }
        };

        match agentd_ingest(&self.agentd_socket, &body_str).await {
            Ok(_) => {
                tracing::debug!(action = %receipt.action, wallet_id = %receipt.wallet_id, "receipt logged");
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to log receipt to agentd (non-fatal)");
            }
        }
    }
}

/// Send an ingest request to agentd over Unix socket.
async fn agentd_ingest(socket_path: &str, body: &str) -> Result<()> {
    use hyper::body::Bytes;
    use hyper::Request;
    use hyper_util::rt::TokioIo;
    use http_body_util::Full;
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(socket_path).await?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::warn!(error = %e, "agentd connection error");
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/memory/ingest")
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))?;

    let _resp = sender.send_request(req).await?;
    Ok(())
}
