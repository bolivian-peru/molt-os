use std::sync::Arc;

use chrono::{Duration, Utc};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::knowledge::{insert_observation, prune_observations, Observation};
use crate::TeachdState;

/// Background loop: collects system observations every 30 seconds.
pub async fn observer_loop(state: Arc<Mutex<TeachdState>>, cancel: CancellationToken) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("observer loop shutting down");
                break;
            }
            _ = interval.tick() => {
                collect_cpu_observation(&state).await;
                collect_memory_observation(&state).await;
                collect_service_observation(&state).await;
                collect_journal_observation(&state).await;
                prune_old_observations(&state).await;
            }
        }
    }
}

/// Collect CPU usage from /proc/stat.
async fn collect_cpu_observation(state: &Arc<Mutex<TeachdState>>) {
    let data = match tokio::fs::read_to_string("/proc/stat").await {
        Ok(content) => content,
        Err(_) => {
            // Not on Linux or /proc not available — emit a placeholder
            tracing::debug!("cannot read /proc/stat (not Linux?), skipping CPU observation");
            return;
        }
    };

    // Parse the first "cpu" line for aggregate stats
    if let Some(cpu_line) = data.lines().find(|l| l.starts_with("cpu ")) {
        let fields: Vec<u64> = cpu_line
            .split_whitespace()
            .skip(1)
            .filter_map(|f| f.parse().ok())
            .collect();

        if fields.len() >= 4 {
            let user = fields[0];
            let nice = fields[1];
            let system = fields[2];
            let idle = fields[3];
            let total = user + nice + system + idle;

            let obs = Observation {
                id: uuid::Uuid::new_v4().to_string(),
                ts: Utc::now(),
                source: "cpu".to_string(),
                data: serde_json::json!({
                    "user": user,
                    "nice": nice,
                    "system": system,
                    "idle": idle,
                    "total": total,
                }),
            };

            let st = state.lock().await;
            if let Err(e) = insert_observation(&st.db, &obs) {
                tracing::warn!(error = %e, "failed to store CPU observation");
            }
        }
    }
}

/// Collect memory usage from /proc/meminfo.
async fn collect_memory_observation(state: &Arc<Mutex<TeachdState>>) {
    let data = match tokio::fs::read_to_string("/proc/meminfo").await {
        Ok(content) => content,
        Err(_) => {
            tracing::debug!("cannot read /proc/meminfo, skipping memory observation");
            return;
        }
    };

    let mut mem_total: Option<u64> = None;
    let mut mem_available: Option<u64> = None;
    let mut mem_free: Option<u64> = None;

    for line in data.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            match parts[0] {
                "MemTotal:" => mem_total = parts[1].parse().ok(),
                "MemAvailable:" => mem_available = parts[1].parse().ok(),
                "MemFree:" => mem_free = parts[1].parse().ok(),
                _ => {}
            }
        }
    }

    if let Some(total) = mem_total {
        let obs = Observation {
            id: uuid::Uuid::new_v4().to_string(),
            ts: Utc::now(),
            source: "memory".to_string(),
            data: serde_json::json!({
                "total_kb": total,
                "available_kb": mem_available,
                "free_kb": mem_free,
            }),
        };

        let st = state.lock().await;
        if let Err(e) = insert_observation(&st.db, &obs) {
            tracing::warn!(error = %e, "failed to store memory observation");
        }
    }
}

/// Collect systemd service state summary.
async fn collect_service_observation(state: &Arc<Mutex<TeachdState>>) {
    let output = match tokio::process::Command::new("systemctl")
        .args(["list-units", "--type=service", "--no-pager", "--no-legend", "--plain"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => {
            tracing::debug!("systemctl not available, skipping service observation");
            return;
        }
    };

    if !output.status.success() {
        return;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut running = 0u32;
    let mut failed = 0u32;
    let mut total = 0u32;
    let mut failed_names: Vec<String> = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 4 {
            total += 1;
            match parts[3] {
                "running" => running += 1,
                "failed" => {
                    failed += 1;
                    failed_names.push(parts[0].to_string());
                }
                _ => {}
            }
        }
    }

    let obs = Observation {
        id: uuid::Uuid::new_v4().to_string(),
        ts: Utc::now(),
        source: "service".to_string(),
        data: serde_json::json!({
            "total": total,
            "running": running,
            "failed": failed,
            "failed_names": failed_names,
        }),
    };

    let st = state.lock().await;
    if let Err(e) = insert_observation(&st.db, &obs) {
        tracing::warn!(error = %e, "failed to store service observation");
    }
}

/// Collect recent journal errors (last 30s window).
async fn collect_journal_observation(state: &Arc<Mutex<TeachdState>>) {
    let output = match tokio::process::Command::new("journalctl")
        .args(["--since", "30s ago", "--priority=err", "--no-pager", "-o", "json", "--output-fields=MESSAGE,SYSLOG_IDENTIFIER"])
        .output()
        .await
    {
        Ok(o) => o,
        Err(_) => {
            tracing::debug!("journalctl not available, skipping journal observation");
            return;
        }
    };

    if !output.status.success() {
        return;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut errors: Vec<serde_json::Value> = Vec::new();

    for line in stdout.lines() {
        if let Ok(entry) = serde_json::from_str::<serde_json::Value>(line) {
            errors.push(serde_json::json!({
                "identifier": entry.get("SYSLOG_IDENTIFIER").and_then(|v| v.as_str()).unwrap_or("unknown"),
                "message": entry.get("MESSAGE").and_then(|v| v.as_str()).unwrap_or(""),
            }));
        }
    }

    // Only store if there were errors
    if errors.is_empty() {
        return;
    }

    let obs = Observation {
        id: uuid::Uuid::new_v4().to_string(),
        ts: Utc::now(),
        source: "journal".to_string(),
        data: serde_json::json!({
            "error_count": errors.len(),
            "errors": errors,
        }),
    };

    let st = state.lock().await;
    if let Err(e) = insert_observation(&st.db, &obs) {
        tracing::warn!(error = %e, "failed to store journal observation");
    }
}

/// Prune observations older than 7 days.
async fn prune_old_observations(state: &Arc<Mutex<TeachdState>>) {
    let cutoff = (Utc::now() - Duration::days(7)).to_rfc3339();
    let st = state.lock().await;
    match prune_observations(&st.db, &cutoff) {
        Ok(n) if n > 0 => tracing::debug!(pruned = n, "pruned old observations"),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "failed to prune observations"),
    }
}
