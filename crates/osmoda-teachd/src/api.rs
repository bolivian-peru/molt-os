use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::knowledge::{
    self, insert_knowledge_doc, KnowledgeDoc, Observation, Optimization, Pattern,
};
use crate::optimizer;
use crate::teacher;
use crate::TeachdState;

type SharedState = Arc<Mutex<TeachdState>>;

// ── GET /health ──

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub observation_count: u32,
    pub pattern_count: u32,
    pub knowledge_count: u32,
    pub optimization_count: u32,
    pub observer_running: bool,
    pub learner_running: bool,
}

pub async fn health_handler(State(state): State<SharedState>) -> Json<HealthResponse> {
    let st = state.lock().await;
    let obs_count = knowledge::observation_count(&st.db).unwrap_or(0);
    let pat_count = knowledge::pattern_count(&st.db).unwrap_or(0);
    let kd_count = knowledge::knowledge_count(&st.db).unwrap_or(0);
    let opt_count = knowledge::optimization_count(&st.db).unwrap_or(0);

    Json(HealthResponse {
        status: "ok".to_string(),
        observation_count: obs_count,
        pattern_count: pat_count,
        knowledge_count: kd_count,
        optimization_count: opt_count,
        observer_running: true,
        learner_running: true,
    })
}

// ── GET /observations ──

#[derive(Debug, Deserialize)]
pub struct ObservationsQuery {
    pub source: Option<String>,
    pub since: Option<String>,
    pub limit: Option<u32>,
}

pub async fn observations_handler(
    State(state): State<SharedState>,
    Query(query): Query<ObservationsQuery>,
) -> Json<Vec<Observation>> {
    let st = state.lock().await;
    let limit = query.limit.unwrap_or(50);
    let observations = knowledge::list_observations(
        &st.db,
        query.source.as_deref(),
        query.since.as_deref(),
        limit,
    )
    .unwrap_or_default();
    Json(observations)
}

// ── GET /patterns ──

#[derive(Debug, Deserialize)]
pub struct PatternsQuery {
    #[serde(rename = "type")]
    pub pattern_type: Option<String>,
    pub min_confidence: Option<f64>,
}

pub async fn patterns_handler(
    State(state): State<SharedState>,
    Query(query): Query<PatternsQuery>,
) -> Json<Vec<Pattern>> {
    let st = state.lock().await;
    let min_confidence = query.min_confidence.unwrap_or(0.5);
    let patterns =
        knowledge::list_patterns(&st.db, query.pattern_type.as_deref(), min_confidence)
            .unwrap_or_default();
    Json(patterns)
}

// ── GET /knowledge ──

#[derive(Debug, Deserialize)]
pub struct KnowledgeQuery {
    pub category: Option<String>,
    pub tag: Option<String>,
    pub limit: Option<u32>,
}

pub async fn knowledge_list_handler(
    State(state): State<SharedState>,
    Query(query): Query<KnowledgeQuery>,
) -> Json<Vec<KnowledgeDoc>> {
    let st = state.lock().await;
    let limit = query.limit.unwrap_or(20);
    let docs = knowledge::list_knowledge_docs(
        &st.db,
        query.category.as_deref(),
        query.tag.as_deref(),
        limit,
    )
    .unwrap_or_default();
    Json(docs)
}

// ── GET /knowledge/{id} ──

pub async fn knowledge_get_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<KnowledgeDoc>, axum::http::StatusCode> {
    let st = state.lock().await;
    knowledge::get_knowledge_doc(&st.db, &id)
        .ok()
        .flatten()
        .map(Json)
        .ok_or(axum::http::StatusCode::NOT_FOUND)
}

// ── POST /knowledge/create ──

#[derive(Debug, Deserialize)]
pub struct CreateKnowledgeRequest {
    pub title: String,
    pub category: String,
    pub content: String,
    pub tags: Option<Vec<String>>,
}

pub async fn knowledge_create_handler(
    State(state): State<SharedState>,
    Json(body): Json<CreateKnowledgeRequest>,
) -> Result<Json<KnowledgeDoc>, (axum::http::StatusCode, String)> {
    let st = state.lock().await;
    let now = chrono::Utc::now();
    let doc = KnowledgeDoc {
        id: format!("kd-manual-{}", uuid::Uuid::new_v4()),
        title: body.title,
        category: body.category,
        content: body.content,
        source_patterns: Vec::new(),
        confidence: 1.0, // Manual docs are fully confident
        created_at: now,
        updated_at: now,
        applied: false,
        tags: body.tags.unwrap_or_default(),
    };

    insert_knowledge_doc(&st.db, &doc).map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        )
    })?;

    st.receipt_logger
        .log_event("knowledge.create", &doc.id, &doc.title)
        .await;

    Ok(Json(doc))
}

// ── POST /knowledge/{id}/update ──

#[derive(Debug, Deserialize)]
pub struct UpdateKnowledgeRequest {
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub category: Option<String>,
}

pub async fn knowledge_update_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateKnowledgeRequest>,
) -> Result<Json<KnowledgeDoc>, (axum::http::StatusCode, String)> {
    let st = state.lock().await;
    let mut doc = knowledge::get_knowledge_doc(&st.db, &id)
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                e.to_string(),
            )
        })?
        .ok_or((
            axum::http::StatusCode::NOT_FOUND,
            "knowledge doc not found".to_string(),
        ))?;

    if let Some(content) = body.content {
        doc.content = content;
    }
    if let Some(tags) = body.tags {
        doc.tags = tags;
    }
    if let Some(category) = body.category {
        doc.category = category;
    }
    doc.updated_at = chrono::Utc::now();

    insert_knowledge_doc(&st.db, &doc).map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        )
    })?;

    st.receipt_logger
        .log_event("knowledge.update", &doc.id, &doc.title)
        .await;

    Ok(Json(doc))
}

// ── POST /teach ──

#[derive(Debug, Deserialize)]
pub struct TeachRequest {
    pub context: String,
}

#[derive(Debug, Serialize)]
pub struct TeachResponse {
    pub relevant_docs: Vec<KnowledgeDoc>,
    pub injected_tokens: usize,
}

pub async fn teach_handler(
    State(state): State<SharedState>,
    Json(body): Json<TeachRequest>,
) -> Json<TeachResponse> {
    let st = state.lock().await;
    let (docs, tokens) = teacher::teach_context(&st.db, &body.context).unwrap_or((Vec::new(), 0));

    Json(TeachResponse {
        relevant_docs: docs,
        injected_tokens: tokens,
    })
}

// ── POST /optimize/suggest ──

pub async fn optimize_suggest_handler(
    State(state): State<SharedState>,
) -> Result<Json<Vec<Optimization>>, (axum::http::StatusCode, String)> {
    let st = state.lock().await;
    let suggestions = optimizer::suggest_optimizations(&st.db).map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string(),
        )
    })?;

    if !suggestions.is_empty() {
        st.receipt_logger
            .log_event(
                "optimize.suggest",
                "teachd",
                &format!("{} suggestions generated", suggestions.len()),
            )
            .await;
    }

    Ok(Json(suggestions))
}

// ── POST /optimize/approve/{id} ──

pub async fn optimize_approve_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Optimization>, (axum::http::StatusCode, String)> {
    let st = state.lock().await;
    let opt = optimizer::approve_optimization(&st.db, &id).map_err(|e| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            e.to_string(),
        )
    })?;

    st.receipt_logger
        .log_event("optimize.approve", &id, &opt.description)
        .await;

    Ok(Json(opt))
}

// ── POST /optimize/apply/{id} ──

pub async fn optimize_apply_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Optimization>, (axum::http::StatusCode, String)> {
    // Extract what we need from the lock, then drop it before async work
    let (watch_socket, receipt_logger, opt) = {
        let st = state.lock().await;
        let watch_socket = st.watch_socket.clone();
        let receipt_logger = st.receipt_logger.clone();
        let opt = knowledge::get_optimization(&st.db, &id)
            .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or((axum::http::StatusCode::NOT_FOUND, "optimization not found".to_string()))?;

        if opt.status != knowledge::OptStatus::Approved {
            return Err((
                axum::http::StatusCode::BAD_REQUEST,
                format!("optimization {} is in status {:?}, expected Approved", id, opt.status),
            ));
        }
        (watch_socket, receipt_logger, opt)
    };

    // Async work: create SafeSwitch + execute action (no lock held)
    let result = optimizer::execute_and_apply(&opt, &watch_socket).await;

    // Re-acquire lock to update DB
    let st = state.lock().await;
    let updated_opt = match result {
        Ok(switch_id) => {
            knowledge::update_optimization_status(&st.db, &id, &knowledge::OptStatus::Applied, Some(&switch_id))
                .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let mut o = opt;
            o.status = knowledge::OptStatus::Applied;
            o.switch_id = Some(switch_id);
            o
        }
        Err(e) => {
            tracing::error!(opt_id = %id, error = %e, "optimization action failed");
            knowledge::update_optimization_status(&st.db, &id, &knowledge::OptStatus::RolledBack, None)
                .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            let mut o = opt;
            o.status = knowledge::OptStatus::RolledBack;
            o
        }
    };

    receipt_logger
        .log_event("optimize.apply", &id, &format!("status={}", updated_opt.status))
        .await;

    Ok(Json(updated_opt))
}

// ── GET /optimizations ──

#[derive(Debug, Deserialize)]
pub struct OptimizationsQuery {
    pub status: Option<String>,
    pub limit: Option<u32>,
}

pub async fn optimizations_list_handler(
    State(state): State<SharedState>,
    Query(query): Query<OptimizationsQuery>,
) -> Json<Vec<Optimization>> {
    let st = state.lock().await;
    let limit = query.limit.unwrap_or(20);
    let optimizations =
        knowledge::list_optimizations(&st.db, query.status.as_deref(), limit).unwrap_or_default();
    Json(optimizations)
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_response_serialization() {
        let health = HealthResponse {
            status: "ok".to_string(),
            observation_count: 100,
            pattern_count: 5,
            knowledge_count: 3,
            optimization_count: 1,
            observer_running: true,
            learner_running: true,
        };

        let json = serde_json::to_value(&health).expect("serialize");
        assert_eq!(json["status"], "ok");
        assert_eq!(json["observation_count"], 100);
        assert_eq!(json["pattern_count"], 5);
        assert_eq!(json["knowledge_count"], 3);
        assert_eq!(json["optimization_count"], 1);
        assert!(json["observer_running"].as_bool().unwrap());
        assert!(json["learner_running"].as_bool().unwrap());
    }

    #[test]
    fn test_teach_response_serialization() {
        let resp = TeachResponse {
            relevant_docs: Vec::new(),
            injected_tokens: 0,
        };

        let json = serde_json::to_value(&resp).expect("serialize");
        assert!(json["relevant_docs"].is_array());
        assert_eq!(json["injected_tokens"], 0);
    }
}
