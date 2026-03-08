use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::knowledge::{
    self, insert_knowledge_doc, KnowledgeDoc, Observation, Optimization, Pattern,
    AgentAction, SkillCandidate, SkillExecution,
};
use crate::optimizer;
use crate::skillgen;
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
    pub action_count: u32,
    pub skill_candidate_count: u32,
    pub skill_execution_count: u32,
    pub observer_running: bool,
    pub learner_running: bool,
    pub skillgen_running: bool,
}

pub async fn health_handler(State(state): State<SharedState>) -> Json<HealthResponse> {
    let st = state.lock().await;
    let obs_count = knowledge::observation_count(&st.db).unwrap_or(0);
    let pat_count = knowledge::pattern_count(&st.db).unwrap_or(0);
    let kd_count = knowledge::knowledge_count(&st.db).unwrap_or(0);
    let opt_count = knowledge::optimization_count(&st.db).unwrap_or(0);
    let act_count = knowledge::agent_action_count(&st.db).unwrap_or(0);
    let sc_count = knowledge::skill_candidate_count(&st.db).unwrap_or(0);
    let se_count = knowledge::skill_execution_count(&st.db).unwrap_or(0);

    Json(HealthResponse {
        status: "ok".to_string(),
        observation_count: obs_count,
        pattern_count: pat_count,
        knowledge_count: kd_count,
        optimization_count: opt_count,
        action_count: act_count,
        skill_candidate_count: sc_count,
        skill_execution_count: se_count,
        observer_running: true,
        learner_running: true,
        skillgen_running: true,
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

// ── POST /observe/action ──

#[derive(Debug, Deserialize)]
pub struct ObserveActionRequest {
    pub tool: String,
    pub params: Option<serde_json::Value>,
    pub result_summary: Option<String>,
    pub context: Option<String>,
    pub session_id: Option<String>,
    pub success: Option<bool>,
}

pub async fn observe_action_handler(
    State(state): State<SharedState>,
    Json(body): Json<ObserveActionRequest>,
) -> Result<Json<AgentAction>, (axum::http::StatusCode, String)> {
    let st = state.lock().await;
    let now = chrono::Utc::now();
    let action = AgentAction {
        id: format!("act-{}", uuid::Uuid::new_v4()),
        ts: now,
        session_id: body.session_id.unwrap_or_else(|| "default".to_string()),
        tool: body.tool,
        params: body.params.unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
        result_summary: body.result_summary,
        context: body.context,
        success: body.success.unwrap_or(true),
    };

    knowledge::insert_agent_action(&st.db, &action).map_err(|e| {
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    Ok(Json(action))
}

// ── GET /actions ──

#[derive(Debug, Deserialize)]
pub struct ActionsQuery {
    pub tool: Option<String>,
    pub session_id: Option<String>,
    pub since: Option<String>,
    pub limit: Option<u32>,
}

pub async fn actions_list_handler(
    State(state): State<SharedState>,
    Query(query): Query<ActionsQuery>,
) -> Json<Vec<AgentAction>> {
    let st = state.lock().await;
    let limit = query.limit.unwrap_or(50);
    let actions = knowledge::list_agent_actions(
        &st.db,
        query.tool.as_deref(),
        query.session_id.as_deref(),
        query.since.as_deref(),
        limit,
    ).unwrap_or_default();
    Json(actions)
}

// ── POST /skills/detect ──

#[derive(Debug, Serialize)]
pub struct DetectResponse {
    pub sequences_found: usize,
    pub new_candidates: usize,
    pub candidates: Vec<SkillCandidate>,
}

pub async fn skill_detect_handler(
    State(state): State<SharedState>,
) -> Result<Json<DetectResponse>, (axum::http::StatusCode, String)> {
    skillgen::run_skillgen_cycle(&state).await.map_err(|e| {
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    // Return the current candidates
    let st = state.lock().await;
    let candidates = knowledge::list_skill_candidates(&st.db, None, 20).unwrap_or_default();
    let action_count = knowledge::agent_action_count(&st.db).unwrap_or(0);

    Ok(Json(DetectResponse {
        sequences_found: action_count as usize,
        new_candidates: candidates.iter().filter(|c| c.status == knowledge::SkillCandidateStatus::Pending).count(),
        candidates,
    }))
}

// ── GET /skills/candidates ──

#[derive(Debug, Deserialize)]
pub struct SkillCandidatesQuery {
    pub status: Option<String>,
    pub limit: Option<u32>,
}

pub async fn skill_candidates_handler(
    State(state): State<SharedState>,
    Query(query): Query<SkillCandidatesQuery>,
) -> Json<Vec<SkillCandidate>> {
    let st = state.lock().await;
    let limit = query.limit.unwrap_or(20);
    let candidates = knowledge::list_skill_candidates(
        &st.db,
        query.status.as_deref(),
        limit,
    ).unwrap_or_default();
    Json(candidates)
}

// ── POST /skills/generate/{id} ──

pub async fn skill_generate_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<SkillCandidate>, (axum::http::StatusCode, String)> {
    let st = state.lock().await;
    let mut candidate = knowledge::get_skill_candidate(&st.db, &id)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((axum::http::StatusCode::NOT_FOUND, "candidate not found".to_string()))?;

    if candidate.status != knowledge::SkillCandidateStatus::Pending {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            format!("candidate {} is already in status {}", id, candidate.status),
        ));
    }

    // Write SKILL.md file
    let skill_path = skillgen::write_skill_file(&st.state_dir, &candidate)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    candidate.status = knowledge::SkillCandidateStatus::Generated;
    candidate.skill_path = Some(skill_path);
    candidate.updated_at = chrono::Utc::now();

    knowledge::insert_skill_candidate(&st.db, &candidate)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    st.receipt_logger
        .log_event("skill.generated", &candidate.name, &format!("path={}", candidate.skill_path.as_deref().unwrap_or("?")))
        .await;

    Ok(Json(candidate))
}

// ── POST /skills/promote/{id} ──

pub async fn skill_promote_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<SkillCandidate>, (axum::http::StatusCode, String)> {
    let st = state.lock().await;
    let mut candidate = knowledge::get_skill_candidate(&st.db, &id)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((axum::http::StatusCode::NOT_FOUND, "candidate not found".to_string()))?;

    if candidate.status != knowledge::SkillCandidateStatus::Generated {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            format!("candidate {} must be in 'generated' status to promote (current: {})", id, candidate.status),
        ));
    }

    // Update the SKILL.md to set activation: auto
    if let Some(ref path) = candidate.skill_path {
        if let Ok(content) = std::fs::read_to_string(path) {
            let updated = content.replace("activation: manual", "activation: auto");
            if let Err(e) = std::fs::write(path, &updated) {
                tracing::warn!(error = %e, "failed to update SKILL.md activation mode");
            }
        }
    }

    candidate.status = knowledge::SkillCandidateStatus::Promoted;
    candidate.updated_at = chrono::Utc::now();

    knowledge::insert_skill_candidate(&st.db, &candidate)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    st.receipt_logger
        .log_event("skill.promoted", &candidate.name, "activation set to auto")
        .await;

    Ok(Json(candidate))
}

// ── POST /skills/execution ──

#[derive(Debug, Deserialize)]
pub struct SkillExecutionRequest {
    pub skill_name: String,
    pub session_id: Option<String>,
    pub outcome: String, // "success", "failure", "partial"
    pub notes: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SkillExecutionResponse {
    pub execution: SkillExecution,
    pub success_rate: f64,
    pub total_executions: u32,
}

pub async fn skill_execution_handler(
    State(state): State<SharedState>,
    Json(body): Json<SkillExecutionRequest>,
) -> Result<Json<SkillExecutionResponse>, (axum::http::StatusCode, String)> {
    let st = state.lock().await;
    let now = chrono::Utc::now();
    let exec = SkillExecution {
        id: format!("se-{}", uuid::Uuid::new_v4()),
        skill_name: body.skill_name.clone(),
        session_id: body.session_id.unwrap_or_else(|| "default".to_string()),
        ts: now,
        outcome: body.outcome,
        notes: body.notes,
    };

    knowledge::insert_skill_execution(&st.db, &exec)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let (success, total) = knowledge::skill_success_rate(&st.db, &body.skill_name)
        .unwrap_or((0, 0));
    let success_rate = if total > 0 { success as f64 / total as f64 } else { 0.0 };

    st.receipt_logger
        .log_event(
            "skill.executed",
            &exec.skill_name,
            &format!("outcome={}, rate={:.0}% ({}/{})", exec.outcome, success_rate * 100.0, success, total),
        )
        .await;

    Ok(Json(SkillExecutionResponse {
        execution: exec,
        success_rate,
        total_executions: total,
    }))
}

// ── GET /skills/executions ──

#[derive(Debug, Deserialize)]
pub struct SkillExecutionsQuery {
    pub skill_name: Option<String>,
    pub limit: Option<u32>,
}

pub async fn skill_executions_list_handler(
    State(state): State<SharedState>,
    Query(query): Query<SkillExecutionsQuery>,
) -> Json<Vec<SkillExecution>> {
    let st = state.lock().await;
    let limit = query.limit.unwrap_or(20);
    let execs = knowledge::list_skill_executions(
        &st.db,
        query.skill_name.as_deref(),
        limit,
    ).unwrap_or_default();
    Json(execs)
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
            action_count: 50,
            skill_candidate_count: 2,
            skill_execution_count: 10,
            observer_running: true,
            learner_running: true,
            skillgen_running: true,
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
