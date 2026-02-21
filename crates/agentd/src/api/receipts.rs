use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::state::SharedState;

// ── Receipts ──

#[derive(Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub id: String,
    pub event_id: i64,
    pub receipt_type: String,
    pub actor: String,
    pub summary: String,
    pub detail: serde_json::Value,
    pub timestamp: String,
    pub reversible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_action: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReceiptQuery {
    #[serde(rename = "type")]
    pub receipt_type: Option<String>,
    pub since: Option<String>,
    pub limit: Option<i64>,
}

/// GET /receipts — query structured receipts from the ledger.
pub async fn receipts_handler(
    State(state): State<SharedState>,
    Query(params): Query<ReceiptQuery>,
) -> Result<Json<Vec<Receipt>>, axum::http::StatusCode> {
    let ledger = state.ledger.lock().await;

    let filter = crate::ledger::EventFilter {
        event_type: params.receipt_type.clone(),
        actor: None,
        limit: Some(params.limit.unwrap_or(50).min(500)), // Cap at 500
    };

    let events = ledger.query(&filter).map_err(|e| {
        tracing::error!(error = %e, "failed to query receipts");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut receipts = Vec::new();
    for event in events {
        // Apply since filter
        if let Some(ref since) = params.since {
            if event.ts < *since {
                continue;
            }
        }

        let parsed: serde_json::Value =
            serde_json::from_str(&event.payload).unwrap_or(json!({}));

        let receipt_type = event.event_type.clone();
        let reversible = receipt_type.contains("switch") || receipt_type.contains("send");
        let undo_action = if receipt_type.contains("switch.commit") {
            Some("switch.rollback".to_string())
        } else {
            None
        };

        let summary = parsed
            .get("summary")
            .and_then(|s| s.as_str())
            .or_else(|| parsed.get("content").and_then(|s| s.as_str()))
            .unwrap_or(&event.event_type)
            .to_string();

        receipts.push(Receipt {
            id: format!("receipt-{}", event.id),
            event_id: event.id,
            receipt_type,
            actor: event.actor,
            summary,
            detail: parsed,
            timestamp: event.ts,
            reversible,
            undo_action,
        });
    }

    Ok(Json(receipts))
}

// ── Incident Workspaces (backed by dedicated tables) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentWorkspace {
    pub id: String,
    pub name: String,
    pub status: String,
    pub steps: Vec<IncidentStep>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentStep {
    pub step_number: u32,
    pub action: String,
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt_id: Option<String>,
    pub timestamp: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateIncidentRequest {
    pub name: String,
}

/// POST /incident/create — create a new incident workspace.
pub async fn incident_create_handler(
    State(state): State<SharedState>,
    Json(body): Json<CreateIncidentRequest>,
) -> Result<Json<IncidentWorkspace>, axum::http::StatusCode> {
    let id = uuid::Uuid::new_v4().to_string();

    let ledger = state.ledger.lock().await;

    // Create in dedicated table
    ledger.create_incident(&id, &body.name).map_err(|e| {
        tracing::error!(error = %e, "failed to create incident");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Also log to event ledger for auditability
    let _ = ledger.append(
        "incident.create",
        "agentd",
        &json!({"incident_id": id, "name": body.name}).to_string(),
    );

    let incident = ledger.get_incident(&id).map_err(|e| {
        tracing::error!(error = %e, "failed to read back incident");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let incident = incident.ok_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;

    tracing::info!(incident_id = %id, name = %body.name, "incident workspace created");
    Ok(Json(row_to_workspace(incident)))
}

#[derive(Debug, Deserialize)]
pub struct AddStepRequest {
    pub action: String,
    pub result: String,
    pub receipt_id: Option<String>,
}

/// POST /incident/{id}/step — add a step to an incident workspace (resumable).
pub async fn incident_step_handler(
    State(state): State<SharedState>,
    Path(incident_id): Path<String>,
    Json(body): Json<AddStepRequest>,
) -> Result<Json<IncidentWorkspace>, axum::http::StatusCode> {
    let ledger = state.ledger.lock().await;

    // Verify incident exists and get current step count
    let incident = ledger.get_incident(&incident_id).map_err(|e| {
        tracing::error!(error = %e, "failed to query incident");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let incident = incident.ok_or(axum::http::StatusCode::NOT_FOUND)?;
    let step_number = incident.steps.len() as u32 + 1;

    // Add step
    ledger
        .add_incident_step(
            &incident_id,
            step_number,
            &body.action,
            &body.result,
            body.receipt_id.as_deref(),
        )
        .map_err(|e| {
            tracing::error!(error = %e, "failed to add incident step");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Log to event ledger
    let _ = ledger.append(
        "incident.step",
        "agentd",
        &json!({
            "incident_id": incident_id,
            "step_number": step_number,
            "action": body.action,
            "result": body.result,
        })
        .to_string(),
    );

    // Return updated incident
    let updated = ledger.get_incident(&incident_id).map_err(|e| {
        tracing::error!(error = %e, "failed to read back incident");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(row_to_workspace(
        updated.ok_or(axum::http::StatusCode::INTERNAL_SERVER_ERROR)?,
    )))
}

/// GET /incident/{id} — get full incident workspace with all steps.
pub async fn incident_get_handler(
    State(state): State<SharedState>,
    Path(incident_id): Path<String>,
) -> Result<Json<IncidentWorkspace>, axum::http::StatusCode> {
    let ledger = state.ledger.lock().await;

    let incident = ledger.get_incident(&incident_id).map_err(|e| {
        tracing::error!(error = %e, "failed to query incident");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let incident = incident.ok_or(axum::http::StatusCode::NOT_FOUND)?;
    Ok(Json(row_to_workspace(incident)))
}

#[derive(Debug, Deserialize)]
pub struct IncidentListQuery {
    pub status: Option<String>,
}

/// GET /incidents — list incident workspaces.
pub async fn incidents_list_handler(
    State(state): State<SharedState>,
    Query(params): Query<IncidentListQuery>,
) -> Result<Json<Vec<IncidentWorkspace>>, axum::http::StatusCode> {
    let ledger = state.ledger.lock().await;

    let incidents = ledger
        .list_incidents(params.status.as_deref())
        .map_err(|e| {
            tracing::error!(error = %e, "failed to list incidents");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(incidents.into_iter().map(row_to_workspace).collect()))
}

/// Convert a ledger IncidentRow to the API IncidentWorkspace type.
fn row_to_workspace(row: crate::ledger::IncidentRow) -> IncidentWorkspace {
    IncidentWorkspace {
        id: row.id,
        name: row.name,
        status: row.status,
        created_at: row.created_at,
        resolved_at: row.resolved_at,
        steps: row
            .steps
            .into_iter()
            .map(|s| IncidentStep {
                step_number: s.step_number,
                action: s.action,
                result: s.result,
                receipt_id: s.receipt_id,
                timestamp: s.timestamp,
            })
            .collect(),
    }
}
