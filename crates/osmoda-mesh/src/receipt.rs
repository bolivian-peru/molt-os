use anyhow::Result;
use serde::Serialize;

/// Logs mesh operations to the agentd ledger via HTTP-over-Unix-socket.
/// Same pattern as keyd/receipt.rs.
pub struct ReceiptLogger {
    agentd_socket: String,
}

#[derive(Debug, Serialize)]
pub struct MeshReceipt {
    pub event_type: String,
    pub peer_id: String,
    pub detail: String,
    pub timestamp: String,
}

impl ReceiptLogger {
    pub fn new(agentd_socket: &str) -> Self {
        Self {
            agentd_socket: agentd_socket.to_string(),
        }
    }

    /// Log a mesh event to the agentd ledger. Best-effort — failures are logged but not fatal.
    pub async fn log_receipt(&self, receipt: &MeshReceipt) {
        let payload = match serde_json::to_string(receipt) {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize mesh receipt");
                return;
            }
        };

        let body = serde_json::json!({
            "source": "osmoda-mesh",
            "content": payload,
            "category": format!("mesh.{}", receipt.event_type),
            "tags": ["mesh", &receipt.event_type, &receipt.peer_id],
        });

        let body_str = match serde_json::to_string(&body) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize mesh receipt body");
                return;
            }
        };

        match agentd_ingest(&self.agentd_socket, &body_str).await {
            Ok(_) => {
                tracing::debug!(
                    event_type = %receipt.event_type,
                    peer_id = %receipt.peer_id,
                    "mesh receipt logged"
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to log mesh receipt to agentd (non-fatal)");
            }
        }
    }

    /// Log a peer connect event.
    pub async fn log_connect(&self, peer_id: &str) {
        self.log_receipt(&MeshReceipt {
            event_type: "peer.connect".to_string(),
            peer_id: peer_id.to_string(),
            detail: "peer connected".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
        .await;
    }

    /// Log a peer disconnect event.
    pub async fn log_disconnect(&self, peer_id: &str) {
        self.log_receipt(&MeshReceipt {
            event_type: "peer.disconnect".to_string(),
            peer_id: peer_id.to_string(),
            detail: "peer disconnected".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
        .await;
    }

    /// Log a message sent event.
    pub async fn log_message_sent(&self, peer_id: &str, msg_type: &str) {
        self.log_receipt(&MeshReceipt {
            event_type: "message.sent".to_string(),
            peer_id: peer_id.to_string(),
            detail: format!("sent {} message", msg_type),
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
        .await;
    }

    /// Log a message received event.
    pub async fn log_message_received(&self, peer_id: &str, msg_type: &str) {
        self.log_receipt(&MeshReceipt {
            event_type: "message.received".to_string(),
            peer_id: peer_id.to_string(),
            detail: format!("received {} message", msg_type),
            timestamp: chrono::Utc::now().to_rfc3339(),
        })
        .await;
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
