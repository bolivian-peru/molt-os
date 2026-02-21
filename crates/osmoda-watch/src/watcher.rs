use serde::{Deserialize, Serialize};

use crate::switch::HealthCheck;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Watcher {
    pub id: String,
    pub name: String,
    pub check: HealthCheck,
    pub interval_secs: u64,
    pub actions: Vec<WatchAction>,
    pub state: WatcherState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WatchAction {
    RestartService { unit: String },
    RollbackGeneration,
    Notify { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WatcherState {
    Healthy,
    Degraded { since: String, retries: u32 },
}

impl Watcher {
    pub fn is_degraded(&self) -> bool {
        matches!(self.state, WatcherState::Degraded { .. })
    }
}

/// Execute a single watcher action.
pub async fn execute_action(action: &WatchAction) -> Result<String, String> {
    // Validate before executing (defense-in-depth)
    crate::validate::validate_watch_action(action)?;

    match action {
        WatchAction::RestartService { unit } => {
            let output = tokio::process::Command::new("systemctl")
                .args(["restart", unit])
                .output()
                .await
                .map_err(|e| format!("failed to restart {unit}: {e}"))?;

            if output.status.success() {
                Ok(format!("restarted {unit}"))
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(format!("restart {unit} failed: {stderr}"))
            }
        }
        WatchAction::RollbackGeneration => {
            match crate::switch::rollback_generation().await {
                Ok(gen) => Ok(format!("rolled back to {gen}")),
                Err(e) => Err(format!("rollback failed: {e}")),
            }
        }
        WatchAction::Notify { message } => {
            tracing::warn!(message = %message, "watcher notification");
            Ok(format!("notified: {message}"))
        }
    }
}

/// Run the watcher check and escalation loop for a single watcher.
/// Returns the updated watcher state and any actions taken.
pub async fn run_watcher_cycle(watcher: &mut Watcher) -> Vec<String> {
    let mut actions_taken = Vec::new();

    let (passed, failures) = crate::switch::run_health_checks(&[watcher.check.clone()]).await;

    if passed {
        if watcher.is_degraded() {
            tracing::info!(name = %watcher.name, "watcher recovered");
            actions_taken.push(format!("{}: recovered", watcher.name));
        }
        watcher.state = WatcherState::Healthy;
        return actions_taken;
    }

    // Health check failed — escalate
    let retries = match &watcher.state {
        WatcherState::Healthy => {
            watcher.state = WatcherState::Degraded {
                since: chrono::Utc::now().to_rfc3339(),
                retries: 0,
            };
            0
        }
        WatcherState::Degraded { since, retries } => {
            let new_retries = retries + 1;
            watcher.state = WatcherState::Degraded {
                since: since.clone(),
                retries: new_retries,
            };
            new_retries
        }
    };

    tracing::warn!(
        name = %watcher.name,
        retries = retries,
        failures = ?failures,
        "watcher check failed"
    );

    // Execute actions in escalation order (based on retry count)
    let action_idx = (retries as usize).min(watcher.actions.len().saturating_sub(1));
    if let Some(action) = watcher.actions.get(action_idx) {
        match execute_action(action).await {
            Ok(msg) => {
                actions_taken.push(format!("{}: {msg}", watcher.name));
            }
            Err(msg) => {
                actions_taken.push(format!("{}: action failed — {msg}", watcher.name));
            }
        }
    }

    actions_taken
}
