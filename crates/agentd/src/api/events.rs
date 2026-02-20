use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::Value;

use crate::ledger::EventFilter;
use crate::state::SharedState;

/// Query parameters for the events log endpoint.
#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub actor: Option<String>,
    pub limit: Option<i64>,
}

pub async fn events_log_handler(
    State(state): State<SharedState>,
    Query(params): Query<EventsQuery>,
) -> Result<Json<Vec<Value>>, axum::http::StatusCode> {
    let ledger = state.ledger.lock().await;

    let filter = EventFilter {
        event_type: params.event_type,
        actor: params.actor,
        limit: Some(params.limit.unwrap_or(50)),
    };

    let events = ledger.query(&filter).map_err(|e| {
        tracing::error!(error = %e, "failed to query events");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let events_json: Vec<Value> = events
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "ts": e.ts,
                "type": e.event_type,
                "actor": e.actor,
                "payload": e.payload,
                "prev_hash": e.prev_hash,
                "hash": e.hash,
            })
        })
        .collect();

    Ok(Json(events_json))
}
