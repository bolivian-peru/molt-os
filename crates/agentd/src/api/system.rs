use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sysinfo::{Disks, System};

use crate::state::SharedState;

#[derive(Debug, Deserialize)]
pub struct SystemQuery {
    pub query: String,
    pub args: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct SystemQueryResponse {
    pub query: String,
    pub result: Value,
}

#[derive(Debug, Serialize)]
struct ProcessInfo {
    pid: u32,
    name: String,
    cpu_usage: f32,
    memory: u64,
    status: String,
    start_time: u64,
}

pub async fn system_query_handler(
    State(state): State<SharedState>,
    Json(body): Json<SystemQuery>,
) -> Result<Json<SystemQueryResponse>, axum::http::StatusCode> {
    let result = match body.query.as_str() {
        "processes" => query_processes(&state, &body.args).await,
        "disk" => query_disk().await,
        "hostname" => query_hostname(),
        "uptime" => query_uptime(),
        other => {
            tracing::warn!(query = other, "unknown system query");
            Ok(json!({ "error": format!("unknown query: {other}") }))
        }
    };

    let result = match result {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, query = %body.query, "system query failed");
            return Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        }
    };

    // Log the query to the ledger
    {
        let ledger = state.ledger.lock().await;
        let payload = serde_json::to_string(&json!({
            "query": &body.query,
            "args": &body.args,
        }))
        .unwrap_or_default();

        if let Err(e) = ledger.append("system.query", "agentd", &payload) {
            tracing::error!(error = %e, "failed to log system query to ledger");
        }
    }

    Ok(Json(SystemQueryResponse {
        query: body.query,
        result,
    }))
}

async fn query_processes(
    state: &SharedState,
    args: &Option<Value>,
) -> anyhow::Result<Value> {
    let mut sys = state.sys.lock().await;
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let mut processes: Vec<ProcessInfo> = sys
        .processes()
        .iter()
        .map(|(pid, proc_)| ProcessInfo {
            pid: pid.as_u32(),
            name: proc_.name().to_string_lossy().to_string(),
            cpu_usage: proc_.cpu_usage(),
            memory: proc_.memory(),
            status: format!("{:?}", proc_.status()),
            start_time: proc_.start_time(),
        })
        .collect();

    // Apply sorting from args
    let sort_by = args
        .as_ref()
        .and_then(|a| a.get("sort"))
        .and_then(|s| s.as_str())
        .unwrap_or("cpu");

    match sort_by {
        "memory" => processes.sort_by(|a, b| b.memory.cmp(&a.memory)),
        _ => processes.sort_by(|a, b| {
            b.cpu_usage
                .partial_cmp(&a.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
    }

    // Apply limit from args
    let limit = args
        .as_ref()
        .and_then(|a| a.get("limit"))
        .and_then(|l| l.as_u64())
        .unwrap_or(50) as usize;

    processes.truncate(limit);

    Ok(serde_json::to_value(processes)?)
}

async fn query_disk() -> anyhow::Result<Value> {
    let disks = Disks::new_with_refreshed_list();

    let disk_info: Vec<Value> = disks
        .iter()
        .map(|d| {
            let total = d.total_space();
            let available = d.available_space();
            let used = total.saturating_sub(available);
            json!({
                "mount_point": d.mount_point().to_string_lossy(),
                "filesystem": d.file_system().to_string_lossy(),
                "total": total,
                "used": used,
                "available": available,
            })
        })
        .collect();

    Ok(json!(disk_info))
}

fn query_hostname() -> anyhow::Result<Value> {
    let hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());
    Ok(json!({ "hostname": hostname }))
}

fn query_uptime() -> anyhow::Result<Value> {
    let uptime = System::uptime();
    Ok(json!({ "uptime_seconds": uptime }))
}
