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
    let max_results = body.max_results.unwrap_or(10).min(100);

    // Try FTS5 first, fall back to keyword scan if it fails
    let fts_results = ledger.fts_search(&body.query, max_results * 2);

    let chunks = match fts_results {
        Ok(results) if !results.is_empty() => {
            let mut chunks: Vec<MemoryChunk> = Vec::new();
            // Normalize BM25 scores to 0.0-1.0 range
            let max_score = results.iter().map(|(_, s)| *s).fold(0.0_f64, f64::max);

            for (event, score) in &results {
                if let Some(ref tf) = body.timeframe {
                    if !event.ts.starts_with(tf) {
                        continue;
                    }
                }

                let relevance = if max_score > 0.0 { score / max_score } else { 0.0 };
                let parsed: serde_json::Value =
                    serde_json::from_str(&event.payload).unwrap_or(json!({}));

                let content = parsed.get("content").and_then(|c| c.as_str())
                    .or_else(|| parsed.get("detail").and_then(|d| d.as_str()))
                    .or_else(|| parsed.get("summary").and_then(|s| s.as_str()))
                    .unwrap_or(&event.payload)
                    .to_string();

                let category = parsed.get("category").and_then(|c| c.as_str()).map(|s| s.to_string());
                let tags = parsed.get("tags").and_then(|t| {
                    t.as_array().map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                });

                chunks.push(MemoryChunk { id: event.id, ts: event.ts.clone(), content, category, tags, relevance });
            }
            chunks.truncate(max_results);
            chunks
        }
        Ok(_) => Vec::new(), // FTS5 returned nothing
        Err(e) => {
            // FTS5 failed — fall back to keyword scan
            tracing::warn!(error = %e, "FTS5 search failed, falling back to keyword scan");
            keyword_scan_fallback(&ledger, &body, max_results)?
        }
    };

    let total_searched = ledger.event_count().unwrap_or(0) as usize;

    Ok(Json(MemoryRecallResponse {
        query: body.query,
        chunks,
        total_searched,
    }))
}

/// Fallback keyword scan when FTS5 is unavailable.
fn keyword_scan_fallback(
    ledger: &crate::ledger::Ledger,
    body: &MemoryRecallRequest,
    max_results: usize,
) -> Result<Vec<MemoryChunk>, axum::http::StatusCode> {
    let all_events = ledger
        .query(&EventFilter { event_type: Some("memory.store".to_string()), actor: None, limit: Some(5000) })
        .map_err(|e| { tracing::error!(error = %e, "keyword fallback query failed"); axum::http::StatusCode::INTERNAL_SERVER_ERROR })?;

    let query_lower = body.query.to_lowercase();
    let query_terms: Vec<&str> = query_lower.split_whitespace().collect();
    let mut chunks: Vec<MemoryChunk> = Vec::new();

    for event in &all_events {
        if let Some(ref tf) = body.timeframe {
            if !event.ts.starts_with(tf) { continue; }
        }
        let payload_lower = event.payload.to_lowercase();
        let matching = query_terms.iter().filter(|t| payload_lower.contains(**t)).count();
        if matching == 0 { continue; }

        let relevance = matching as f64 / query_terms.len() as f64;
        let parsed: serde_json::Value = serde_json::from_str(&event.payload).unwrap_or(json!({}));
        let content = parsed.get("content").and_then(|c| c.as_str())
            .or_else(|| parsed.get("detail").and_then(|d| d.as_str()))
            .or_else(|| parsed.get("summary").and_then(|s| s.as_str()))
            .unwrap_or(&event.payload).to_string();
        let category = parsed.get("category").and_then(|c| c.as_str()).map(|s| s.to_string());
        let tags = parsed.get("tags").and_then(|t| {
            t.as_array().map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        });

        chunks.push(MemoryChunk { id: event.id, ts: event.ts.clone(), content, category, tags, relevance });
    }

    chunks.sort_by(|a, b| b.relevance.partial_cmp(&a.relevance).unwrap_or(std::cmp::Ordering::Equal).then(b.id.cmp(&a.id)));
    chunks.truncate(max_results);
    Ok(chunks)
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
