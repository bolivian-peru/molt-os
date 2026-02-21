use std::sync::Arc;

use axum::extract::{Path, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::routine::{Routine, RoutineAction, Trigger, validate_unit_name};
use crate::RoutinesState;

type SharedState = Arc<Mutex<RoutinesState>>;

// ── POST /routine/add ──

#[derive(Debug, Deserialize)]
pub struct AddRoutineRequest {
    pub name: String,
    pub trigger: Trigger,
    pub action: RoutineAction,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

pub async fn routine_add_handler(
    State(state): State<SharedState>,
    Json(body): Json<AddRoutineRequest>,
) -> Result<Json<Routine>, (axum::http::StatusCode, String)> {
    // Validate action at registration time (reject dangerous inputs early)
    validate_routine_action(&body.action)?;

    let id = uuid::Uuid::new_v4().to_string();
    let routine = Routine {
        id: id.clone(),
        name: body.name,
        trigger: body.trigger,
        action: body.action,
        enabled: body.enabled,
        last_run: None,
        run_count: 0,
    };

    let mut st = state.lock().await;
    st.routines.push(routine.clone());

    // Persist to disk
    if let Err(e) = save_routines(&st.routines, &st.routines_dir) {
        tracing::warn!(error = %e, "failed to persist routine");
    }

    tracing::info!(routine_id = %id, name = %routine.name, "routine added");
    Ok(Json(routine))
}

/// Validate a routine action before allowing it to be registered.
fn validate_routine_action(action: &RoutineAction) -> Result<(), (axum::http::StatusCode, String)> {
    let bad = |msg: String| (axum::http::StatusCode::BAD_REQUEST, msg);
    match action {
        RoutineAction::Command { cmd, .. } => {
            crate::routine::validate_command(cmd).map_err(bad)?;
        }
        RoutineAction::Webhook { url, .. } => {
            crate::routine::validate_webhook_url(url).map_err(bad)?;
        }
        RoutineAction::ServiceMonitor { units } => {
            for unit in units {
                validate_unit_name(unit).map_err(bad)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// ── GET /routine/list ──

pub async fn routine_list_handler(State(state): State<SharedState>) -> Json<Vec<Routine>> {
    let st = state.lock().await;
    Json(st.routines.clone())
}

// ── DELETE /routine/remove/{id} ──

pub async fn routine_remove_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, axum::http::StatusCode> {
    let mut st = state.lock().await;
    let before = st.routines.len();
    st.routines.retain(|r| r.id != id);
    if st.routines.len() < before {
        if let Err(e) = save_routines(&st.routines, &st.routines_dir) {
            tracing::warn!(error = %e, "failed to persist after removal");
        }
        tracing::info!(routine_id = %id, "routine removed");
        Ok(Json(serde_json::json!({"removed": id})))
    } else {
        Err(axum::http::StatusCode::NOT_FOUND)
    }
}

// ── POST /routine/trigger/{id} ──

pub async fn routine_trigger_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let mut st = state.lock().await;
    let routine = st
        .routines
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or((
            axum::http::StatusCode::NOT_FOUND,
            "routine not found".to_string(),
        ))?;

    let action = routine.action.clone();
    let name = routine.name.clone();
    routine.last_run = Some(chrono::Utc::now().to_rfc3339());
    routine.run_count += 1;

    // Release the lock before executing
    drop(st);

    let result = crate::routine::execute_action(&action).await;
    match &result {
        Ok(output) => {
            tracing::info!(routine = %name, "manual trigger succeeded");
            Ok(Json(serde_json::json!({"status": "ok", "output": output})))
        }
        Err(err) => {
            tracing::warn!(routine = %name, error = %err, "manual trigger failed");
            Ok(Json(serde_json::json!({"status": "error", "error": err})))
        }
    }
}

// ── GET /routine/history ──

#[derive(Debug, Serialize)]
pub struct RoutineHistoryEntry {
    pub id: String,
    pub name: String,
    pub last_run: Option<String>,
    pub run_count: u64,
    pub enabled: bool,
}

pub async fn routine_history_handler(
    State(state): State<SharedState>,
) -> Json<Vec<RoutineHistoryEntry>> {
    let st = state.lock().await;
    let history: Vec<RoutineHistoryEntry> = st
        .routines
        .iter()
        .map(|r| RoutineHistoryEntry {
            id: r.id.clone(),
            name: r.name.clone(),
            last_run: r.last_run.clone(),
            run_count: r.run_count,
            enabled: r.enabled,
        })
        .collect();
    Json(history)
}

// ── GET /health ──

#[derive(Debug, Serialize)]
pub struct RoutinesHealthResponse {
    pub status: String,
    pub routine_count: usize,
    pub enabled_count: usize,
}

pub async fn health_handler(State(state): State<SharedState>) -> Json<RoutinesHealthResponse> {
    let st = state.lock().await;
    let enabled = st.routines.iter().filter(|r| r.enabled).count();
    Json(RoutinesHealthResponse {
        status: "ok".to_string(),
        routine_count: st.routines.len(),
        enabled_count: enabled,
    })
}

// ── Persistence helpers ──

fn save_routines(routines: &[Routine], dir: &str) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = std::path::Path::new(dir).join("routines.json");
    let data = serde_json::to_string_pretty(routines)?;
    std::fs::write(path, data)?;
    Ok(())
}

pub fn load_routines(dir: &str) -> Vec<Routine> {
    let path = std::path::Path::new(dir).join("routines.json");
    if path.exists() {
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(routines) = serde_json::from_str::<Vec<Routine>>(&data) {
                return routines;
            }
        }
    }
    // Return defaults if no persisted routines
    crate::routine::default_routines()
}

#[cfg(test)]
mod tests {
    use super::{load_routines, save_routines};
    use crate::routine::{default_routines, Routine, RoutineAction, Trigger};

    fn make_routine(id: &str, name: &str) -> Routine {
        Routine {
            id: id.to_string(),
            name: name.to_string(),
            trigger: Trigger::Interval { seconds: 60 },
            action: RoutineAction::HealthCheck,
            enabled: true,
            last_run: None,
            run_count: 0,
        }
    }

    #[test]
    fn test_load_routines_empty_dir_returns_defaults() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let routines = load_routines(dir.path().to_str().unwrap());
        let defaults = default_routines();
        assert_eq!(
            routines.len(),
            defaults.len(),
            "expected default routines when no routines.json is present"
        );
    }

    #[test]
    fn test_save_and_load_routines_roundtrip() {
        let dir = tempfile::tempdir().expect("failed to create tempdir");
        let dir_str = dir.path().to_str().unwrap();

        let routines = vec![
            make_routine("r-1", "Morning Briefing"),
            make_routine("r-2", "Nightly Cleanup"),
        ];

        save_routines(&routines, dir_str).expect("save_routines should succeed");

        let loaded = load_routines(dir_str);
        assert_eq!(loaded.len(), 2, "expected 2 routines after roundtrip");

        let names: Vec<&str> = loaded.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"Morning Briefing"), "Morning Briefing missing");
        assert!(names.contains(&"Nightly Cleanup"), "Nightly Cleanup missing");
    }
}
