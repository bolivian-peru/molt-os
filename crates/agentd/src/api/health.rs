use axum::extract::State;
use axum::Json;
use serde::Serialize;
use sysinfo::{Disks, System};

use crate::state::SharedState;

#[derive(Serialize)]
pub struct DiskInfo {
    pub mount_point: String,
    pub total: u64,
    pub used: u64,
    pub available: u64,
}

#[derive(Serialize)]
pub struct LoadAverage {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub cpu_usage: Vec<f32>,
    pub memory_total: u64,
    pub memory_used: u64,
    pub memory_available: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    pub load_average: LoadAverage,
    pub uptime: u64,
    pub hostname: String,
    pub disks: Vec<DiskInfo>,
}

pub async fn health_handler(State(state): State<SharedState>) -> Json<HealthResponse> {
    let mut sys = state.sys.lock().await;

    // Refresh all relevant subsystems
    sys.refresh_cpu_all();
    sys.refresh_memory();

    let cpu_usage: Vec<f32> = sys.cpus().iter().map(|cpu| cpu.cpu_usage()).collect();

    let memory_total = sys.total_memory();
    let memory_used = sys.used_memory();
    let memory_available = sys.available_memory();
    let swap_total = sys.total_swap();
    let swap_used = sys.used_swap();

    let load_avg = System::load_average();
    let load_average = LoadAverage {
        one: load_avg.one,
        five: load_avg.five,
        fifteen: load_avg.fifteen,
    };

    let uptime = System::uptime();
    let hostname = System::host_name().unwrap_or_else(|| "unknown".to_string());

    let disks = Disks::new_with_refreshed_list();
    let disk_info: Vec<DiskInfo> = disks
        .iter()
        .map(|d| {
            let total = d.total_space();
            let available = d.available_space();
            let used = total.saturating_sub(available);
            DiskInfo {
                mount_point: d.mount_point().to_string_lossy().to_string(),
                total,
                used,
                available,
            }
        })
        .collect();

    Json(HealthResponse {
        status: "ok".to_string(),
        cpu_usage,
        memory_total,
        memory_used,
        memory_available,
        swap_total,
        swap_used,
        load_average,
        uptime,
        hostname,
        disks: disk_info,
    })
}
