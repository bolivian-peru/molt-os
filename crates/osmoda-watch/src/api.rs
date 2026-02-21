use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::switch::{HealthCheck, SwitchSession, SwitchStatus};
use crate::watcher::{WatchAction, Watcher, WatcherState};
use crate::WatchState;

type SharedState = Arc<Mutex<WatchState>>;

// ── SafeSwitch endpoints ──

#[derive(Debug, Deserialize)]
pub struct BeginSwitchRequest {
    pub plan: String,
    pub ttl_secs: u64,
    pub health_checks: Vec<HealthCheck>,
}

#[derive(Debug, Serialize)]
pub struct BeginSwitchResponse {
    pub id: String,
    pub previous_generation: String,
    pub status: String,
}

/// POST /switch/begin — start a deploy transaction with health checks + TTL.
pub async fn switch_begin_handler(
    State(state): State<SharedState>,
    Json(body): Json<BeginSwitchRequest>,
) -> Result<Json<BeginSwitchResponse>, (axum::http::StatusCode, String)> {
    // Validate all health checks at registration time
    for check in &body.health_checks {
        crate::validate::validate_health_check(check).map_err(|e| {
            (axum::http::StatusCode::BAD_REQUEST, e)
        })?;
    }

    let id = uuid::Uuid::new_v4().to_string();
    let previous_generation = crate::switch::current_generation().map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("failed to get current generation: {e}"),
        )
    })?;

    let session = SwitchSession {
        id: id.clone(),
        plan: body.plan,
        ttl_secs: body.ttl_secs,
        health_checks: body.health_checks,
        started_at: chrono::Utc::now().to_rfc3339(),
        previous_generation: previous_generation.clone(),
        status: SwitchStatus::Probation,
    };

    let mut st = state.lock().await;
    st.switches.insert(id.clone(), session);

    tracing::info!(switch_id = %id, "SafeSwitch session started (probation)");

    Ok(Json(BeginSwitchResponse {
        id,
        previous_generation,
        status: "probation".to_string(),
    }))
}

/// GET /switch/status/{id} — check the status of a switch session.
pub async fn switch_status_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<SwitchSession>, axum::http::StatusCode> {
    let st = state.lock().await;
    st.switches
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or(axum::http::StatusCode::NOT_FOUND)
}

/// POST /switch/commit/{id} — manually commit a switch session.
pub async fn switch_commit_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<SwitchSession>, (axum::http::StatusCode, String)> {
    let mut st = state.lock().await;
    let session = st.switches.get_mut(&id).ok_or((
        axum::http::StatusCode::NOT_FOUND,
        "switch session not found".to_string(),
    ))?;

    if !session.is_active() {
        return Err((
            axum::http::StatusCode::CONFLICT,
            "switch session is not in probation".to_string(),
        ));
    }

    session.status = SwitchStatus::Committed {
        committed_at: chrono::Utc::now().to_rfc3339(),
    };

    tracing::info!(switch_id = %id, "SafeSwitch committed");
    Ok(Json(session.clone()))
}

/// POST /switch/rollback/{id} — manually rollback a switch session.
pub async fn switch_rollback_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<SwitchSession>, (axum::http::StatusCode, String)> {
    let mut st = state.lock().await;
    let session = st.switches.get_mut(&id).ok_or((
        axum::http::StatusCode::NOT_FOUND,
        "switch session not found".to_string(),
    ))?;

    if !session.is_active() {
        return Err((
            axum::http::StatusCode::CONFLICT,
            "switch session is not in probation".to_string(),
        ));
    }

    // Perform rollback
    if let Err(e) = crate::switch::rollback_generation().await {
        tracing::error!(error = %e, "rollback failed");
    }

    session.status = SwitchStatus::RolledBack {
        reason: "manual rollback".to_string(),
        rolled_back_at: chrono::Utc::now().to_rfc3339(),
    };

    tracing::info!(switch_id = %id, "SafeSwitch rolled back (manual)");
    Ok(Json(session.clone()))
}

// ── Watcher endpoints ──

#[derive(Debug, Deserialize)]
pub struct AddWatcherRequest {
    pub name: String,
    pub check: HealthCheck,
    pub interval_secs: Option<u64>,
    pub actions: Vec<WatchAction>,
}

/// POST /watcher/add — add a new autopilot watcher.
pub async fn watcher_add_handler(
    State(state): State<SharedState>,
    Json(body): Json<AddWatcherRequest>,
) -> Result<Json<Watcher>, (axum::http::StatusCode, String)> {
    // Validate health check at registration time
    crate::validate::validate_health_check(&body.check).map_err(|e| {
        (axum::http::StatusCode::BAD_REQUEST, e)
    })?;

    // Validate all watcher actions
    for action in &body.actions {
        crate::validate::validate_watch_action(action).map_err(|e| {
            (axum::http::StatusCode::BAD_REQUEST, e)
        })?;
    }

    let id = uuid::Uuid::new_v4().to_string();
    let watcher = Watcher {
        id: id.clone(),
        name: body.name,
        check: body.check,
        interval_secs: body.interval_secs.unwrap_or(30),
        actions: body.actions,
        state: WatcherState::Healthy,
    };

    let mut st = state.lock().await;
    st.watchers.push(watcher.clone());

    if let Err(e) = save_watchers(&st.watchers, &st.data_dir) {
        tracing::warn!(error = %e, "failed to persist watcher");
    }

    tracing::info!(watcher_id = %id, name = %watcher.name, "watcher added");
    Ok(Json(watcher))
}

/// GET /watcher/list — list all active watchers.
pub async fn watcher_list_handler(State(state): State<SharedState>) -> Json<Vec<Watcher>> {
    let st = state.lock().await;
    Json(st.watchers.clone())
}

/// DELETE /watcher/remove/{id} — remove a watcher.
pub async fn watcher_remove_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    let mut st = state.lock().await;
    let before = st.watchers.len();
    st.watchers.retain(|w| w.id != id);
    if st.watchers.len() < before {
        if let Err(e) = save_watchers(&st.watchers, &st.data_dir) {
            tracing::warn!(error = %e, "failed to persist after watcher removal");
        }
        tracing::info!(watcher_id = %id, "watcher removed");
        Ok(Json(serde_json::json!({"removed": id})))
    } else {
        Err(axum::http::StatusCode::NOT_FOUND)
    }
}

// ── Persistence helpers ──

fn save_watchers(watchers: &[Watcher], dir: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = std::path::Path::new(dir).join("watchers.json");
    let data = serde_json::to_string_pretty(watchers)?;
    std::fs::write(path, data)?;
    Ok(())
}

pub fn load_watchers(dir: &str) -> Vec<Watcher> {
    let path = std::path::Path::new(dir).join("watchers.json");
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(watchers) = serde_json::from_str::<Vec<Watcher>>(&data) {
                return watchers;
            }
        }
    }
    Vec::new()
}

// ── Health ──

#[derive(Debug, Serialize)]
pub struct WatchHealthResponse {
    pub status: String,
    pub active_switches: usize,
    pub watchers: usize,
}

pub async fn health_handler(State(state): State<SharedState>) -> Json<WatchHealthResponse> {
    let st = state.lock().await;
    let active = st.switches.values().filter(|s| s.is_active()).count();
    Json(WatchHealthResponse {
        status: "ok".to_string(),
        active_switches: active,
        watchers: st.watchers.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::{load_watchers, save_watchers};
    use crate::switch::HealthCheck;
    use crate::watcher::{WatchAction, Watcher, WatcherState};

    fn make_watcher(id: &str, name: &str) -> Watcher {
        Watcher {
            id: id.to_string(),
            name: name.to_string(),
            check: HealthCheck::SystemdUnit {
                unit: "sshd".to_string(),
            },
            interval_secs: 30,
            actions: vec![WatchAction::Notify {
                message: "test alert".to_string(),
            }],
            state: WatcherState::Healthy,
        }
    }

    #[test]
    fn test_load_watchers_empty_dir() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let watchers = load_watchers(dir.path().to_str().unwrap());
        assert!(watchers.is_empty(), "expected empty vec from dir with no watchers.json");
    }

    #[test]
    fn test_save_and_load_watchers_roundtrip() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let dir_str = dir.path().to_str().unwrap();

        let watchers = vec![
            make_watcher("id-1", "Alpha Watcher"),
            make_watcher("id-2", "Beta Watcher"),
        ];

        save_watchers(&watchers, dir_str).expect("save_watchers should succeed");

        let loaded = load_watchers(dir_str);
        assert_eq!(loaded.len(), 2, "expected 2 watchers after roundtrip");

        let names: Vec<&str> = loaded.iter().map(|w| w.name.as_str()).collect();
        assert!(names.contains(&"Alpha Watcher"), "Alpha Watcher missing");
        assert!(names.contains(&"Beta Watcher"), "Beta Watcher missing");
    }
}
