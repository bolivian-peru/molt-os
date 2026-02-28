use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::sandbox::{Ring, SandboxConfig};
use crate::state::SharedState;

#[derive(Debug, Deserialize)]
pub struct SandboxExecRequest {
    pub command: String,
    pub ring: Option<String>,
    pub capabilities: Option<Vec<String>>,
    pub timeout_secs: Option<u64>,
    pub fs_read: Option<Vec<String>>,
    pub fs_write: Option<Vec<String>>,
    pub network: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct SandboxExecResponse {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub ring: String,
    pub timed_out: bool,
}

#[derive(Debug, Deserialize)]
pub struct MintCapabilityRequest {
    pub granted_to: String,
    pub permissions: Vec<String>,
    pub ttl_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct VerifyCapabilityRequest {
    pub token: crate::sandbox::CapabilityToken,
}

#[derive(Debug, Serialize)]
pub struct VerifyCapabilityResponse {
    pub valid: bool,
}

/// POST /sandbox/exec — execute a command in a sandbox.
pub async fn sandbox_exec_handler(
    State(state): State<SharedState>,
    Json(req): Json<SandboxExecRequest>,
) -> Result<Json<SandboxExecResponse>, (StatusCode, Json<serde_json::Value>)> {
    let engine = state.sandbox_engine.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "sandbox engine not enabled"})),
        )
    })?;

    let ring = match req.ring.as_deref() {
        Some("ring1") => Ring::Ring1,
        _ => Ring::Ring2,
    };

    let config = SandboxConfig {
        ring,
        capabilities: req.capabilities.unwrap_or_default(),
        timeout_secs: req.timeout_secs.unwrap_or(60),
        memory_limit_mb: 512,
        fs_read: req.fs_read.unwrap_or_default(),
        fs_write: req.fs_write.unwrap_or_default(),
        network: req.network.unwrap_or(false),
    };

    // Log the sandbox execution
    {
        let ledger = state.ledger.lock().await;
        let payload = serde_json::json!({
            "command": req.command,
            "ring": ring.to_string(),
            "network": config.network,
        });
        let _ = ledger.append("sandbox.exec", "agent", &payload.to_string());
    }

    match engine.spawn_sandboxed(&config, &req.command).await {
        Ok(result) => Ok(Json(SandboxExecResponse {
            exit_code: result.exit_code,
            stdout: result.stdout,
            stderr: result.stderr,
            ring: result.ring.to_string(),
            timed_out: result.timed_out,
        })),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )),
    }
}

/// POST /capability/mint — create a capability token.
pub async fn capability_mint_handler(
    State(state): State<SharedState>,
    Json(req): Json<MintCapabilityRequest>,
) -> Result<Json<crate::sandbox::CapabilityToken>, (StatusCode, Json<serde_json::Value>)> {
    let engine = state.sandbox_engine.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "sandbox engine not enabled"})),
        )
    })?;

    let ttl = req.ttl_secs.unwrap_or(3600);
    let token = engine.mint_capability(&req.granted_to, req.permissions, ttl);

    // Log token minting
    {
        let ledger = state.ledger.lock().await;
        let payload = serde_json::json!({
            "token_id": token.id,
            "granted_to": token.granted_to,
            "permissions": token.permissions,
            "ttl_secs": ttl,
        });
        let _ = ledger.append("capability.mint", "agent", &payload.to_string());
    }

    Ok(Json(token))
}

/// POST /capability/verify — verify a capability token.
pub async fn capability_verify_handler(
    State(state): State<SharedState>,
    Json(req): Json<VerifyCapabilityRequest>,
) -> Result<Json<VerifyCapabilityResponse>, (StatusCode, Json<serde_json::Value>)> {
    let engine = state.sandbox_engine.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "sandbox engine not enabled"})),
        )
    })?;

    match engine.verify_capability(&req.token) {
        Ok(valid) => Ok(Json(VerifyCapabilityResponse { valid })),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": e.to_string()})),
        )),
    }
}
