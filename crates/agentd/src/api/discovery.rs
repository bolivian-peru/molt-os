use axum::extract::State;
use axum::Json;
use serde::Serialize;
use serde_json::json;
use std::collections::HashMap;

use crate::state::SharedState;

#[derive(Debug, Serialize)]
pub struct DiscoveredService {
    pub name: String,
    pub pid: Option<u32>,
    pub port: Option<u16>,
    pub protocol: Option<String>,
    pub detected_as: Option<String>,
    pub health_url: Option<String>,
    pub systemd_unit: Option<String>,
    pub memory_bytes: Option<u64>,
    pub cpu_usage: Option<f32>,
}

#[derive(Debug, Serialize)]
pub struct DiscoveryResponse {
    pub found: Vec<DiscoveredService>,
    pub total_listening_ports: usize,
    pub total_systemd_services: usize,
}

/// GET /system/discover — find all running services, listening ports, systemd units.
pub async fn system_discover_handler(
    State(state): State<SharedState>,
) -> Result<Json<DiscoveryResponse>, axum::http::StatusCode> {
    let listening = parse_ss_output(&run_cmd("ss", &["-tlnp"]));
    let units = parse_systemctl_output(&run_cmd("systemctl", &["list-units", "--type=service", "--state=running", "--no-pager", "--no-legend"]));

    let total_listening_ports = listening.len();
    let total_systemd_services = units.len();

    // Build PID → sysinfo process map
    let mut sys = state.sys.lock().await;
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let proc_map: HashMap<u32, (&sysinfo::Pid, &sysinfo::Process)> = sys
        .processes()
        .iter()
        .map(|(pid, proc_)| (pid.as_u32(), (pid, proc_)))
        .collect();

    // Merge listening ports with systemd units and process info
    let mut services: Vec<DiscoveredService> = Vec::new();
    let mut seen_pids: std::collections::HashSet<u32> = std::collections::HashSet::new();

    for entry in &listening {
        let detected = detect_service_type(&entry.process_name);
        let health_url = guess_health_url(entry.port, &detected);
        let (mem, cpu) = entry.pid.and_then(|p| proc_map.get(&p)).map(|(_, proc_)| {
            (Some(proc_.memory()), Some(proc_.cpu_usage()))
        }).unwrap_or((None, None));

        // Find matching systemd unit
        let unit = units.iter().find(|u| {
            entry.pid.map_or(false, |pid| u.contains(&format!("{pid}")))
                || u.to_lowercase().contains(&entry.process_name.to_lowercase())
        }).cloned();

        if let Some(pid) = entry.pid {
            seen_pids.insert(pid);
        }

        services.push(DiscoveredService {
            name: entry.process_name.clone(),
            pid: entry.pid,
            port: Some(entry.port),
            protocol: Some("tcp".to_string()),
            detected_as: detected,
            health_url,
            systemd_unit: unit,
            memory_bytes: mem,
            cpu_usage: cpu,
        });
    }

    // Add systemd services not already captured via listening ports
    for unit in &units {
        let unit_base = unit.strip_suffix(".service").unwrap_or(unit);
        let already = services.iter().any(|s| s.systemd_unit.as_deref() == Some(unit));
        if !already {
            services.push(DiscoveredService {
                name: unit_base.to_string(),
                pid: None,
                port: None,
                protocol: None,
                detected_as: None,
                health_url: None,
                systemd_unit: Some(unit.clone()),
                memory_bytes: None,
                cpu_usage: None,
            });
        }
    }

    // Log to ledger
    {
        let ledger = state.ledger.lock().await;
        let payload = serde_json::to_string(&json!({
            "services_found": services.len(),
            "listening_ports": total_listening_ports,
            "systemd_services": total_systemd_services,
        })).unwrap_or_default();
        if let Err(e) = ledger.append("system.discover", "agentd", &payload) {
            tracing::error!(error = %e, "failed to log discovery to ledger");
        }
    }

    Ok(Json(DiscoveryResponse {
        found: services,
        total_listening_ports,
        total_systemd_services,
    }))
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(crate) struct ListeningEntry {
    port: u16,
    process_name: String,
    pid: Option<u32>,
}

fn run_cmd(cmd: &str, args: &[&str]) -> String {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

/// Parse `ss -tlnp` output into listening entries.
pub fn parse_ss_output(output: &str) -> Vec<ListeningEntry> {
    let mut entries = Vec::new();
    for line in output.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 6 { continue; }

        // Local address:port is typically column 3
        let local = cols[3];
        let port = local.rsplit(':').next()
            .and_then(|p| p.parse::<u16>().ok());

        let port = match port {
            Some(p) => p,
            None => continue,
        };

        // Process info is in the last column, format: users:(("name",pid=123,fd=4))
        let proc_col = cols.last().unwrap_or(&"");
        let (process_name, pid) = parse_ss_process(proc_col);

        entries.push(ListeningEntry { port, process_name, pid });
    }
    entries
}

fn parse_ss_process(s: &str) -> (String, Option<u32>) {
    // Format: users:(("name",pid=123,fd=4))
    let name = s.split("((\"").nth(1)
        .and_then(|n| n.split('"').next())
        .unwrap_or("unknown")
        .to_string();

    let pid = s.split("pid=").nth(1)
        .and_then(|p| p.split(|c: char| !c.is_ascii_digit()).next())
        .and_then(|p| p.parse::<u32>().ok());

    (name, pid)
}

fn parse_systemctl_output(output: &str) -> Vec<String> {
    output.lines()
        .filter_map(|line| {
            let unit = line.split_whitespace().next()?;
            if unit.ends_with(".service") {
                Some(unit.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Match process names to known service types.
pub fn detect_service_type(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let known: &[(&[&str], &str)] = &[
        (&["nginx"], "nginx"),
        (&["apache", "httpd"], "apache"),
        (&["postgres", "postmaster"], "postgresql"),
        (&["mysql", "mariadbd", "mysqld"], "mysql"),
        (&["redis-server", "redis"], "redis"),
        (&["node", "nodejs", "next-server", "npm"], "node"),
        (&["python", "python3", "gunicorn", "uvicorn"], "python"),
        (&["sshd"], "ssh"),
        (&["docker", "dockerd", "containerd"], "docker"),
        (&["agentd"], "osmoda-agentd"),
        (&["openclaw"], "osmoda-gateway"),
        (&["cloudflared"], "cloudflare-tunnel"),
        (&["tailscaled"], "tailscale"),
    ];
    for (patterns, label) in known {
        if patterns.iter().any(|p| lower.contains(p)) {
            return Some(label.to_string());
        }
    }
    None
}

fn guess_health_url(port: u16, detected: &Option<String>) -> Option<String> {
    match detected.as_deref() {
        Some("nginx" | "apache") => Some(format!("http://127.0.0.1:{port}/")),
        Some("osmoda-agentd") => Some("http://localhost/health (unix socket)".to_string()),
        Some("redis") => None, // Redis uses its own protocol
        Some("postgresql" | "mysql" | "ssh") => None,
        _ if port == 80 || port == 443 || port == 8080 || port == 3000 => {
            Some(format!("http://127.0.0.1:{port}/"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ss_output_typical() {
        let output = r#"State  Recv-Q Send-Q Local Address:Port  Peer Address:Port Process
LISTEN 0      128          0.0.0.0:22         0.0.0.0:*    users:(("sshd",pid=1234,fd=3))
LISTEN 0      511          0.0.0.0:80         0.0.0.0:*    users:(("nginx",pid=5678,fd=6))
LISTEN 0      4096       127.0.0.1:5432       0.0.0.0:*    users:(("postgres",pid=9012,fd=5))
"#;
        let entries = parse_ss_output(output);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].port, 22);
        assert_eq!(entries[0].process_name, "sshd");
        assert_eq!(entries[0].pid, Some(1234));
        assert_eq!(entries[1].port, 80);
        assert_eq!(entries[1].process_name, "nginx");
        assert_eq!(entries[2].port, 5432);
        assert_eq!(entries[2].process_name, "postgres");
    }

    #[test]
    fn test_parse_ss_output_empty() {
        let entries = parse_ss_output("");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_detect_service_type() {
        assert_eq!(detect_service_type("nginx"), Some("nginx".to_string()));
        assert_eq!(detect_service_type("postgres"), Some("postgresql".to_string()));
        assert_eq!(detect_service_type("redis-server"), Some("redis".to_string()));
        assert_eq!(detect_service_type("node"), Some("node".to_string()));
        assert_eq!(detect_service_type("sshd"), Some("ssh".to_string()));
        assert_eq!(detect_service_type("agentd"), Some("osmoda-agentd".to_string()));
        assert_eq!(detect_service_type("randomthing"), None);
    }

    #[test]
    fn test_guess_health_url() {
        assert_eq!(
            guess_health_url(80, &Some("nginx".to_string())),
            Some("http://127.0.0.1:80/".to_string())
        );
        assert_eq!(guess_health_url(5432, &Some("postgresql".to_string())), None);
        assert_eq!(
            guess_health_url(3000, &None),
            Some("http://127.0.0.1:3000/".to_string())
        );
        assert_eq!(guess_health_url(9999, &None), None);
    }
}
