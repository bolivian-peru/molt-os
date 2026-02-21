use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::ledger::EventFilter;
use crate::state::SharedState;

// ── POST /memory/ingest ──

#[derive(Debug, Deserialize)]
pub struct MemoryIngestRequest {
    pub source: Option<String>,
    pub content: String,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct MemoryIngestResponse {
    pub id: i64,
    pub status: String,
}

pub async fn memory_ingest_handler(
    State(state): State<SharedState>,
    Json(body): Json<MemoryIngestRequest>,
) -> Result<Json<MemoryIngestResponse>, axum::http::StatusCode> {
    let payload = serde_json::to_string(&json!({
        "source": body.source,
        "content": body.content,
        "category": body.category,
        "tags": body.tags,
        "metadata": body.metadata,
    }))
    .unwrap_or_default();

    let actor = body
        .source
        .as_deref()
        .unwrap_or("unknown");

    let ledger = state.ledger.lock().await;
    let event = ledger
        .append("memory.ingest", actor, &payload)
        .map_err(|e| {
            tracing::error!(error = %e, "failed to ingest memory event");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(MemoryIngestResponse {
        id: event.id,
        status: "stored".to_string(),
    }))
}

// ── POST /memory/recall ──

#[derive(Debug, Deserialize)]
pub struct MemoryRecallRequest {
    pub query: String,
    pub max_results: Option<usize>,
    pub timeframe: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MemoryChunk {
    pub id: i64,
    pub ts: String,
    pub content: String,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub relevance: f64,
}

#[derive(Debug, Serialize)]
pub struct MemoryRecallResponse {
    pub query: String,
    pub chunks: Vec<MemoryChunk>,
    pub total_searched: usize,
}

pub async fn memory_recall_handler(
    State(state): State<SharedState>,
    Json(body): Json<MemoryRecallRequest>,
) -> Result<Json<MemoryRecallResponse>, axum::http::StatusCode> {
    let ledger = state.ledger.lock().await;

    // Query all memory events (both ingest and store types)
    // Cap internal scan to 5000 events per type (10000 total max)
    let ingest_events = ledger
        .query(&EventFilter {
            event_type: Some("memory.ingest".to_string()),
            actor: None,
            limit: Some(5000),
        })
        .map_err(|e| {
            tracing::error!(error = %e, "failed to query memory ingest events");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let store_events = ledger
        .query(&EventFilter {
            event_type: Some("memory.store".to_string()),
            actor: None,
            limit: Some(5000),
        })
        .map_err(|e| {
            tracing::error!(error = %e, "failed to query memory store events");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let all_events = ingest_events
        .into_iter()
        .chain(store_events.into_iter())
        .collect::<Vec<_>>();

    let total_searched = all_events.len();
    let max_results = body.max_results.unwrap_or(10).min(100); // Cap at 100
    let query_lower = body.query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();

    let mut chunks: Vec<MemoryChunk> = Vec::new();

    for event in &all_events {
        // Apply timeframe filter if specified (prefix match: "2026-02" matches February 2026)
        if let Some(ref tf) = body.timeframe {
            if !event.ts.starts_with(tf) {
                continue;
            }
        }

        let payload_lower = event.payload.to_lowercase();

        // Compute a simple text-match relevance score
        let matching_terms = query_terms
            .iter()
            .filter(|term| payload_lower.contains(**term))
            .count();

        if matching_terms == 0 {
            continue;
        }

        let relevance = matching_terms as f64 / query_terms.len() as f64;

        // Parse the payload to extract structured fields
        let parsed: serde_json::Value =
            serde_json::from_str(&event.payload).unwrap_or(json!({}));

        let content = parsed
            .get("content")
            .and_then(|c| c.as_str())
            .or_else(|| parsed.get("detail").and_then(|d| d.as_str()))
            .or_else(|| parsed.get("summary").and_then(|s| s.as_str()))
            .unwrap_or(&event.payload)
            .to_string();

        let category = parsed
            .get("category")
            .and_then(|c| c.as_str())
            .map(|s| s.to_string());

        let tags = parsed.get("tags").and_then(|t| {
            t.as_array().map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
        });

        chunks.push(MemoryChunk {
            id: event.id,
            ts: event.ts.clone(),
            content,
            category,
            tags,
            relevance,
        });
    }

    // Sort by relevance descending, then by id descending (newest first)
    chunks.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.id.cmp(&a.id))
    });

    chunks.truncate(max_results);

    Ok(Json(MemoryRecallResponse {
        query: body.query,
        chunks,
        total_searched,
    }))
}

// ── POST /memory/store ──

#[derive(Debug, Deserialize)]
pub struct MemoryStoreRequest {
    pub summary: String,
    pub detail: Option<String>,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct MemoryStoreResponse {
    pub id: i64,
}

pub async fn memory_store_handler(
    State(state): State<SharedState>,
    Json(body): Json<MemoryStoreRequest>,
) -> Result<Json<MemoryStoreResponse>, axum::http::StatusCode> {
    let payload = serde_json::to_string(&json!({
        "summary": body.summary,
        "detail": body.detail,
        "category": body.category,
        "tags": body.tags,
    }))
    .unwrap_or_default();

    let ledger = state.ledger.lock().await;
    let event = ledger
        .append("memory.store", "agentd", &payload)
        .map_err(|e| {
            tracing::error!(error = %e, "failed to store memory event");
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(MemoryStoreResponse { id: event.id }))
}

// ── GET /memory/health ──

#[derive(Debug, Serialize)]
pub struct MemoryHealthResponse {
    pub status: String,
    pub event_count: i64,
    pub state_dir: String,
}

pub async fn memory_health_handler(
    State(state): State<SharedState>,
) -> Result<Json<MemoryHealthResponse>, axum::http::StatusCode> {
    let ledger = state.ledger.lock().await;

    let event_count = ledger.event_count().map_err(|e| {
        tracing::error!(error = %e, "failed to count events for memory health");
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(MemoryHealthResponse {
        status: "ok".to_string(),
        event_count,
        state_dir: state.state_dir.clone(),
    }))
}
