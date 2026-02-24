use anyhow::Result;

/// Logs teachd events to the agentd ledger via HTTP-over-Unix-socket.
/// Same pattern as mcpd receipt.rs.
#[derive(Clone)]
pub struct ReceiptLogger {
    pub agentd_socket: String,
}

impl ReceiptLogger {
    pub fn new(agentd_socket: &str) -> Self {
        Self {
            agentd_socket: agentd_socket.to_string(),
        }
    }

    /// Log a teachd event to the agentd ledger. Best-effort — failures are logged but not fatal.
    pub async fn log_event(&self, event_type: &str, subject: &str, detail: &str) {
        let body = serde_json::json!({
            "source": "osmoda-teachd",
            "content": serde_json::json!({
                "event_type": event_type,
                "subject": subject,
                "detail": detail,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            }).to_string(),
            "category": format!("teach.{}", event_type),
            "tags": ["teach", event_type, subject],
        });

        let body_str = match serde_json::to_string(&body) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize teachd receipt body");
                return;
            }
        };

        match agentd_ingest(&self.agentd_socket, &body_str).await {
            Ok(_) => {
                tracing::debug!(
                    event_type = %event_type,
                    subject = %subject,
                    "teachd receipt logged"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to log teachd receipt to agentd (non-fatal)");
            }
        }
    }
}

/// Send an ingest request to agentd over Unix socket.
async fn agentd_ingest(socket_path: &str, body: &str) -> Result<()> {
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
