use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::fleet::{FleetPhase, FleetSwitch};
use crate::switch::HealthCheck;
use crate::WatchState;

type SharedState = Arc<Mutex<WatchState>>;

#[derive(Debug, Deserialize)]
pub struct FleetProposeRequest {
    pub plan: String,
    pub peer_ids: Vec<String>,
    pub health_checks: Option<Vec<HealthCheck>>,
    pub quorum_percent: Option<u8>,
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct FleetVoteRequest {
    pub peer_id: String,
    pub approve: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct FleetSwitchResponse {
    pub id: String,
    pub plan: String,
    pub proposer: String,
    pub phase: String,
    pub participant_count: usize,
    pub approve_count: usize,
    pub deny_count: usize,
    pub quorum_required: usize,
    pub has_quorum: bool,
    pub result_summary: Option<String>,
}

impl From<&FleetSwitch> for FleetSwitchResponse {
    fn from(fs: &FleetSwitch) -> Self {
        Self {
            id: fs.id.clone(),
            plan: fs.plan.clone(),
            proposer: fs.proposer.clone(),
            phase: format!("{:?}", fs.phase).to_lowercase(),
            participant_count: fs.participant_count(),
            approve_count: fs.approve_count(),
            deny_count: fs.deny_count(),
            quorum_required: fs.quorum_required(),
            has_quorum: fs.has_quorum(),
            result_summary: fs.result_summary.clone(),
        }
    }
}

/// POST /fleet/propose — initiate a fleet-wide switch.
pub async fn fleet_propose_handler(
    State(state): State<SharedState>,
    Json(req): Json<FleetProposeRequest>,
) -> Result<(StatusCode, Json<FleetSwitchResponse>), (StatusCode, Json<serde_json::Value>)> {
    let mut st = state.lock().await;

    let coordinator = st.fleet_coordinator.as_mut().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "fleet coordinator not enabled"})),
        )
    })?;

    if req.peer_ids.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "peer_ids must not be empty"})),
        ));
    }

    let fs = coordinator.propose(
        &req.plan,
        "local",
        req.peer_ids,
        req.health_checks.unwrap_or_default(),
        req.quorum_percent,
        req.timeout_secs,
    );

    // Log to agentd
    let payload = serde_json::json!({
        "fleet_switch_id": fs.id,
        "plan": fs.plan,
        "participants": fs.participant_count(),
    })
    .to_string();
    let sock = st.agentd_socket.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::agentd_post_event(&sock, "fleet.propose", &payload).await {
            tracing::debug!(error = %e, "failed to log fleet.propose (non-fatal)");
        }
    });

    Ok((StatusCode::CREATED, Json(FleetSwitchResponse::from(&fs))))
}

/// GET /fleet/status/{id} — get fleet switch status.
pub async fn fleet_status_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<FleetSwitchResponse>, (StatusCode, Json<serde_json::Value>)> {
    let st = state.lock().await;

    let coordinator = st.fleet_coordinator.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "fleet coordinator not enabled"})),
        )
    })?;

    let fs = coordinator.get(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "fleet switch not found"})),
        )
    })?;

    Ok(Json(FleetSwitchResponse::from(&*fs)))
}

/// POST /fleet/vote/{id} — cast a vote on a fleet switch.
pub async fn fleet_vote_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(req): Json<FleetVoteRequest>,
) -> Result<Json<FleetSwitchResponse>, (StatusCode, Json<serde_json::Value>)> {
    let mut st = state.lock().await;

    let coordinator = st.fleet_coordinator.as_mut().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "fleet coordinator not enabled"})),
        )
    })?;

    let fs = coordinator.get_mut(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "fleet switch not found"})),
        )
    })?;

    if !matches!(fs.phase, FleetPhase::Propose) {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "fleet switch is not in proposal phase"})),
        ));
    }

    if !fs.record_vote(&req.peer_id, req.approve, req.reason) {
        return Err((
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "duplicate vote or not a participant"})),
        ));
    }

    // Check for auto-advance or veto
    if fs.has_quorum() {
        fs.advance_to_execute();
    } else if fs.is_vetoed() {
        fs.abort("proposal vetoed — insufficient approvals possible");
    }

    Ok(Json(FleetSwitchResponse::from(&*fs)))
}

/// POST /fleet/rollback/{id} — force rollback a fleet switch.
pub async fn fleet_rollback_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<FleetSwitchResponse>, (StatusCode, Json<serde_json::Value>)> {
    let mut st = state.lock().await;
    let sock = st.agentd_socket.clone();

    let coordinator = st.fleet_coordinator.as_mut().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "fleet coordinator not enabled"})),
        )
    })?;

    let fs = coordinator.get_mut(&id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({"error": "fleet switch not found"})),
        )
    })?;

    fs.rollback("manual rollback requested");
    let response = FleetSwitchResponse::from(&*fs);

    // Log to agentd
    let payload = serde_json::json!({
        "fleet_switch_id": id,
        "reason": "manual_rollback",
    })
    .to_string();
    tokio::spawn(async move {
        if let Err(e) = crate::agentd_post_event(&sock, "fleet.rollback", &payload).await {
            tracing::debug!(error = %e, "failed to log fleet.rollback (non-fatal)");
        }
    });

    Ok(Json(response))
}
