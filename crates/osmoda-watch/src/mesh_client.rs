use anyhow::Result;
use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;

const MESH_SOCKET: &str = "/run/osmoda/mesh.sock";

/// HTTP-over-Unix-socket client for the mesh daemon.
pub struct MeshClient {
    socket_path: String,
}

impl MeshClient {
    pub fn new(socket_path: &str) -> Self {
        Self {
            socket_path: socket_path.to_string(),
        }
    }

    #[allow(dead_code)]
    pub fn default_client() -> Self {
        Self::new(MESH_SOCKET)
    }

    /// Send a message to a specific peer via the mesh daemon.
    #[allow(dead_code)]
    pub async fn send_to_peer(&self, peer_id: &str, message: &serde_json::Value) -> Result<String> {
        let body = serde_json::json!({
            "message": message,
        });
        self.post(&format!("/peer/{peer_id}/send"), &body).await
    }

    /// Get list of connected peers.
    #[allow(dead_code)]
    pub async fn get_peers(&self) -> Result<String> {
        self.get("/peers").await
    }

    /// Send a message to a room (broadcast to all room members).
    #[allow(dead_code)]
    pub async fn send_to_room(&self, room_id: &str, text: &str) -> Result<String> {
        let body = serde_json::json!({
            "room_id": room_id,
            "text": text,
        });
        self.post("/room/send", &body).await
    }

    async fn get(&self, path: &str) -> Result<String> {
        let stream = UnixStream::connect(&self.socket_path).await?;
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                tracing::debug!(error = %e, "mesh connection closed");
            }
        });

        let req = Request::builder()
            .method("GET")
            .uri(path)
            .body(Full::new(Bytes::new()))?;

        let resp = sender.send_request(req).await?;
        let collected = resp.into_body().collect().await?;
        Ok(String::from_utf8_lossy(&collected.to_bytes()).to_string())
    }

    async fn post(&self, path: &str, body: &serde_json::Value) -> Result<String> {
        let body_str = serde_json::to_string(body)?;

        let stream = UnixStream::connect(&self.socket_path).await?;
        let io = TokioIo::new(stream);
        let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
        tokio::spawn(async move {
            if let Err(e) = conn.await {
                tracing::debug!(error = %e, "mesh connection closed");
            }
        });

        let req = Request::builder()
            .method("POST")
            .uri(path)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(body_str)))?;

        let resp = sender.send_request(req).await?;
        let collected = resp.into_body().collect().await?;
        Ok(String::from_utf8_lossy(&collected.to_bytes()).to_string())
    }
}
