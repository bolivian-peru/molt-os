use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::state::SharedState;

#[derive(Debug, Deserialize)]
pub struct ApprovalRequest {
    pub command: String,
    pub actor: Option<String>,
    pub reason: String,
    pub ttl_secs: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ApprovalDecision {
    pub decided_by: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ApprovalResponse {
    pub id: String,
    pub command: String,
    pub actor: String,
    pub reason: String,
    pub status: String,
    pub created_at: String,
    pub expires_at: String,
    pub decided_at: Option<String>,
    pub decided_by: Option<String>,
    pub is_destructive: bool,
}

impl From<crate::approval::PendingApproval> for ApprovalResponse {
    fn from(a: crate::approval::PendingApproval) -> Self {
        Self {
            id: a.id,
            command: a.command,
            actor: a.actor,
            reason: a.reason,
            status: a.status.to_string(),
            created_at: a.created_at,
            expires_at: a.expires_at,
            decided_at: a.decided_at,
            decided_by: a.decided_by,
            is_destructive: true,
        }
    }
}

/// POST /approval/request — request approval for a destructive operation.
pub async fn approval_request_handler(
    State(state): State<SharedState>,
    Json(req): Json<ApprovalRequest>,
) -> Result<(StatusCode, Json<ApprovalResponse>), (StatusCode, Json<serde_json::Value>)> {
    let gate = state.approval_gate.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "approval gate not enabled"})),
        )
    })?;

    let actor = req.actor.as_deref().unwrap_or("agent");
    let is_destructive = gate.is_destructive(&req.command);

    if !is_destructive {
        // Not destructive — return immediately with auto-approved status
        return Ok((
            StatusCode::OK,
            Json(ApprovalResponse {
                id: String::new(),
                command: req.command,
                actor: actor.to_string(),
                reason: req.reason,
                status: "auto_approved".to_string(),
                created_at: chrono::Utc::now().to_rfc3339(),
                expires_at: String::new(),
                decided_at: Some(chrono::Utc::now().to_rfc3339()),
                decided_by: Some("system".to_string()),
                is_destructive: false,
            }),
        ));
    }

    match gate.request_approval(&req.command, actor, &req.reason, req.ttl_secs) {
        Ok(approval) => {
            // Log to ledger
            let payload = serde_json::json!({
                "approval_id": approval.id,
                "command": approval.command,
                "reason": approval.reason,
            });
            let ledger = state.ledger.lock().await;
            let _ = ledger.append(
                "approval.requested",
                actor,
                &payload.to_string(),
            );

            Ok((StatusCode::CREATED, Json(approval.into())))
        }
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )),
    }
}

/// GET /approval/pending — list all pending approvals.
pub async fn approval_pending_handler(
    State(state): State<SharedState>,
) -> Result<Json<Vec<ApprovalResponse>>, (StatusCode, Json<serde_json::Value>)> {
    let gate = state.approval_gate.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "approval gate not enabled"})),
        )
    })?;

    match gate.list_pending() {
        Ok(approvals) => Ok(Json(approvals.into_iter().map(Into::into).collect())),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )),
    }
}

/// POST /approval/{id}/approve — approve a pending request.
pub async fn approval_approve_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(decision): Json<ApprovalDecision>,
) -> Result<Json<ApprovalResponse>, (StatusCode, Json<serde_json::Value>)> {
    let gate = state.approval_gate.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "approval gate not enabled"})),
        )
    })?;

    let decided_by = decision.decided_by.as_deref().unwrap_or("user");

    match gate.approve(&id, decided_by) {
        Ok(approval) => {
            // Log to ledger
            let payload = serde_json::json!({
                "approval_id": id,
                "command": approval.command,
                "decided_by": decided_by,
            });
            let ledger = state.ledger.lock().await;
            let _ = ledger.append(
                "approval.approved",
                decided_by,
                &payload.to_string(),
            );

            Ok(Json(approval.into()))
        }
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )),
    }
}

/// POST /approval/{id}/deny — deny a pending request.
pub async fn approval_deny_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(decision): Json<ApprovalDecision>,
) -> Result<Json<ApprovalResponse>, (StatusCode, Json<serde_json::Value>)> {
    let gate = state.approval_gate.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "approval gate not enabled"})),
        )
    })?;

    let decided_by = decision.decided_by.as_deref().unwrap_or("user");

    match gate.deny(&id, decided_by) {
        Ok(approval) => {
            let payload = serde_json::json!({
                "approval_id": id,
                "command": approval.command,
                "decided_by": decided_by,
            });
            let ledger = state.ledger.lock().await;
            let _ = ledger.append(
                "approval.denied",
                decided_by,
                &payload.to_string(),
            );

            Ok(Json(approval.into()))
        }
        Err(e) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": e.to_string()})),
        )),
    }
}

/// GET /approval/{id} — check status of an approval request.
pub async fn approval_check_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<ApprovalResponse>, (StatusCode, Json<serde_json::Value>)> {
    let gate = state.approval_gate.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "approval gate not enabled"})),
        )
    })?;

    match gate.check_approval(&id) {
        Ok(Some(approval)) => Ok(Json(approval.into())),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "approval not found"})),
        )),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )),
    }
}
