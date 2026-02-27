use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::state::SharedState;

const DEFAULT_BACKUP_DIR: &str = "/var/backups/osmoda";
const MAX_BACKUPS: usize = 7;

// ── POST /backup/create ──

#[derive(Debug, Serialize)]
pub struct BackupCreateResponse {
    pub backup_id: String,
    pub path: String,
    pub size_bytes: u64,
    pub created_at: String,
}

pub async fn backup_create_handler(
    State(state): State<SharedState>,
) -> Result<Json<BackupCreateResponse>, (axum::http::StatusCode, String)> {
    let backup_dir = std::env::var("OSMODA_BACKUP_DIR")
        .unwrap_or_else(|_| DEFAULT_BACKUP_DIR.to_string());

    std::fs::create_dir_all(&backup_dir).map_err(|e| {
        tracing::error!(error = %e, "failed to create backup directory");
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "backup directory unavailable".to_string())
    })?;

    let now = chrono::Utc::now();
    let timestamp = now.format("%Y%m%d-%H%M%S").to_string();
    let backup_id = format!("backup-{timestamp}");
    let backup_path = format!("{backup_dir}/{backup_id}.tar.gz");

    // First, checkpoint the SQLite WAL for consistent snapshot
    {
        let ledger = state.ledger.lock().await;
        if let Err(e) = ledger.flush() {
            tracing::warn!(error = %e, "WAL checkpoint failed before backup, continuing anyway");
        }
    }

    // Create tar.gz of state directory (excluding cache)
    let state_dir = &state.state_dir;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        tokio::process::Command::new("tar")
            .args([
                "czf", &backup_path,
                "--exclude=*/cache/*",
                "--exclude=*.tmp",
                "-C", "/",
                state_dir.trim_start_matches('/'),
            ])
            .output()
    ).await
    .map_err(|_| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "backup timed out after 120s".to_string()))?
    .map_err(|e| {
        tracing::error!(error = %e, "tar command failed to execute");
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "backup failed".to_string())
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!(stderr = %stderr, "tar backup failed");
        return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, "backup failed".to_string()));
    }

    let size_bytes = std::fs::metadata(&backup_path)
        .map(|m| m.len())
        .unwrap_or(0);

    // Log backup event
    {
        let ledger = state.ledger.lock().await;
        let _ = ledger.append(
            "backup.create",
            "agentd",
            &serde_json::json!({
                "backup_id": backup_id,
                "path": backup_path,
                "size_bytes": size_bytes,
            }).to_string(),
        );
    }

    // Prune old backups (keep MAX_BACKUPS most recent)
    prune_backups(&backup_dir, MAX_BACKUPS);

    tracing::info!(backup_id = %backup_id, path = %backup_path, size_bytes, "backup created");

    Ok(Json(BackupCreateResponse {
        backup_id,
        path: backup_path,
        size_bytes,
        created_at: now.to_rfc3339(),
    }))
}

// ── GET /backup/list ──

#[derive(Debug, Serialize)]
pub struct BackupInfo {
    pub backup_id: String,
    pub path: String,
    pub size_bytes: u64,
    pub created_at: String,
}

pub async fn backup_list_handler() -> Json<Vec<BackupInfo>> {
    let backup_dir = std::env::var("OSMODA_BACKUP_DIR")
        .unwrap_or_else(|_| DEFAULT_BACKUP_DIR.to_string());

    let mut backups = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&backup_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("backup-") && name.ends_with(".tar.gz") {
                let backup_id = name.trim_end_matches(".tar.gz").to_string();
                let meta = entry.metadata().ok();
                let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let created_at = meta
                    .and_then(|m| m.modified().ok())
                    .map(|t| {
                        chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339()
                    })
                    .unwrap_or_default();

                backups.push(BackupInfo {
                    backup_id,
                    path: entry.path().to_string_lossy().to_string(),
                    size_bytes,
                    created_at,
                });
            }
        }
    }

    // Sort newest first
    backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Json(backups)
}

// ── POST /backup/restore ──

#[derive(Debug, Deserialize)]
pub struct BackupRestoreRequest {
    pub backup_id: String,
}

#[derive(Debug, Serialize)]
pub struct BackupRestoreResponse {
    pub restored_from: String,
    pub status: String,
}

pub async fn backup_restore_handler(
    State(state): State<SharedState>,
    Json(body): Json<BackupRestoreRequest>,
) -> Result<Json<BackupRestoreResponse>, (axum::http::StatusCode, String)> {
    let backup_dir = std::env::var("OSMODA_BACKUP_DIR")
        .unwrap_or_else(|_| DEFAULT_BACKUP_DIR.to_string());

    // Validate backup_id — no path traversal
    if body.backup_id.contains("..") || body.backup_id.contains('/') {
        return Err((axum::http::StatusCode::BAD_REQUEST, "invalid backup_id".to_string()));
    }

    let backup_path = format!("{backup_dir}/{}.tar.gz", body.backup_id);
    if !std::path::Path::new(&backup_path).exists() {
        return Err((axum::http::StatusCode::NOT_FOUND, format!("backup not found: {}", body.backup_id)));
    }

    // Log restore intent
    {
        let ledger = state.ledger.lock().await;
        let _ = ledger.append(
            "backup.restore",
            "agentd",
            &serde_json::json!({
                "backup_id": body.backup_id,
                "path": backup_path,
            }).to_string(),
        );
    }

    // Extract backup over the state directory
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        tokio::process::Command::new("tar")
            .args(["xzf", &backup_path, "-C", "/"])
            .output()
    ).await
    .map_err(|_| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "restore timed out after 120s".to_string()))?
    .map_err(|e| {
        tracing::error!(error = %e, "tar extract command failed to execute");
        (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "restore failed".to_string())
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!(stderr = %stderr, "tar restore failed");
        return Err((axum::http::StatusCode::INTERNAL_SERVER_ERROR, "restore failed".to_string()));
    }

    tracing::info!(backup_id = %body.backup_id, "backup restored");

    Ok(Json(BackupRestoreResponse {
        restored_from: body.backup_id,
        status: "restored".to_string(),
    }))
}

/// Remove old backups, keeping only the `keep` most recent.
fn prune_backups(backup_dir: &str, keep: usize) {
    let mut entries: Vec<_> = match std::fs::read_dir(backup_dir) {
        Ok(e) => e.flatten()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.starts_with("backup-") && name.ends_with(".tar.gz")
            })
            .collect(),
        Err(_) => return,
    };

    if entries.len() <= keep {
        return;
    }

    // Sort by filename (which includes timestamp) — newest last
    entries.sort_by_key(|e| e.file_name());

    // Remove oldest entries
    let to_remove = entries.len() - keep;
    for entry in entries.into_iter().take(to_remove) {
        let path = entry.path();
        tracing::info!(path = %path.display(), "pruning old backup");
        let _ = std::fs::remove_file(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prune_backups_noop_when_under_limit() {
        let dir = tempfile::tempdir().unwrap();
        // Create 3 "backups"
        for i in 1..=3 {
            std::fs::write(
                dir.path().join(format!("backup-2026010{i}-120000.tar.gz")),
                "fake",
            ).unwrap();
        }
        prune_backups(dir.path().to_str().unwrap(), 5);
        let count = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_prune_backups_removes_oldest() {
        let dir = tempfile::tempdir().unwrap();
        for i in 1..=5 {
            std::fs::write(
                dir.path().join(format!("backup-2026010{i}-120000.tar.gz")),
                "fake",
            ).unwrap();
        }
        prune_backups(dir.path().to_str().unwrap(), 3);
        let remaining: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(remaining.len(), 3);
        // Oldest two should be gone
        assert!(!remaining.contains(&"backup-20260101-120000.tar.gz".to_string()));
        assert!(!remaining.contains(&"backup-20260102-120000.tar.gz".to_string()));
    }
}
