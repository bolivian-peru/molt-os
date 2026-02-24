use anyhow::{bail, Context, Result};
use chrono::Utc;

use crate::knowledge::{
    get_optimization, insert_optimization, list_knowledge_docs,
    update_optimization_status, KnowledgeDoc, OptAction, OptStatus, Optimization,
};

/// Generate optimization suggestions from unapplied knowledge docs.
pub fn suggest_optimizations(db: &rusqlite::Connection) -> Result<Vec<Optimization>> {
    let docs = list_knowledge_docs(db, None, None, 50)?;
    let unapplied: Vec<&KnowledgeDoc> = docs.iter().filter(|d| !d.applied).collect();

    let mut suggestions = Vec::new();

    for doc in unapplied {
        let (description, action) = suggest_action_for_doc(doc);
        if let Some(action) = action {
            let opt = Optimization {
                id: format!("opt-{}", doc.id),
                knowledge_doc_id: doc.id.clone(),
                description,
                action,
                status: OptStatus::Suggested,
                switch_id: None,
                created_at: Utc::now(),
            };
            insert_optimization(db, &opt)?;
            suggestions.push(opt);
        }
    }

    Ok(suggestions)
}

/// Approve an optimization (changes status from Suggested to Approved).
pub fn approve_optimization(db: &rusqlite::Connection, opt_id: &str) -> Result<Optimization> {
    let opt = get_optimization(db, opt_id)?
        .context("optimization not found")?;

    if opt.status != OptStatus::Suggested {
        bail!(
            "optimization {} is in status {:?}, expected Suggested",
            opt_id,
            opt.status
        );
    }

    update_optimization_status(db, opt_id, &OptStatus::Approved, None)?;

    let mut updated = opt;
    updated.status = OptStatus::Approved;
    Ok(updated)
}

/// Execute the async portion of applying an optimization: SafeSwitch + action execution.
/// Returns Ok(switch_id) on success, Err on failure (caller should rollback status).
pub async fn execute_and_apply(
    opt: &Optimization,
    watch_socket: &str,
) -> Result<String> {
    // Create SafeSwitch session via watch daemon
    let switch_id = create_safe_switch(watch_socket, opt).await?;

    // Execute the action
    match execute_action(&opt.action).await {
        Ok(_) => Ok(switch_id),
        Err(e) => {
            // Rollback the SafeSwitch
            rollback_safe_switch(watch_socket, &switch_id).await.ok();
            Err(e)
        }
    }
}

/// Generate a suggested action based on a knowledge document's content and category.
fn suggest_action_for_doc(doc: &KnowledgeDoc) -> (String, Option<OptAction>) {
    // Extract service names from the title/content for service-related issues
    let title_lower = doc.title.to_lowercase();

    if title_lower.contains("recurring failures") {
        // Extract service name from title pattern "X recurring failures"
        let service_name = title_lower
            .replace("recurring failures", "")
            .trim()
            .to_string();
        if !service_name.is_empty() {
            return (
                format!("Restart {} to recover from recurring failures", service_name),
                Some(OptAction::ServiceRestart {
                    name: service_name,
                }),
            );
        }
    }

    if title_lower.contains("memory") && doc.category == "performance" {
        return (
            "Consider investigating memory usage patterns and potential leaks".to_string(),
            Some(OptAction::Sysctl {
                key: "vm.overcommit_memory".to_string(),
                value: "0".to_string(),
            }),
        );
    }

    // Default: provide a generic suggestion but no auto-action
    (
        format!("Review knowledge doc '{}' for potential optimizations", doc.title),
        None,
    )
}

/// Create a SafeSwitch session via the watch daemon.
async fn create_safe_switch(watch_socket: &str, opt: &Optimization) -> Result<String> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::Request;
    use hyper_util::rt::TokioIo;
    use tokio::net::UnixStream;

    let body = serde_json::json!({
        "plan": format!("teachd optimization: {}", opt.description),
        "ttl_secs": 300,
        "health_checks": [],
    });
    let body_str = serde_json::to_string(&body)?;

    let stream = UnixStream::connect(watch_socket).await
        .context("failed to connect to watch daemon")?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::warn!(error = %e, "watch connection error");
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/switch/begin")
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body_str)))?;

    let resp = sender.send_request(req).await?;
    let body_bytes = http_body_util::BodyExt::collect(resp.into_body())
        .await?
        .to_bytes();
    let resp_json: serde_json::Value = serde_json::from_slice(&body_bytes)?;

    let switch_id = resp_json
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    Ok(switch_id)
}

/// Rollback a SafeSwitch session.
async fn rollback_safe_switch(watch_socket: &str, switch_id: &str) -> Result<()> {
    use http_body_util::Full;
    use hyper::body::Bytes;
    use hyper::Request;
    use hyper_util::rt::TokioIo;
    use tokio::net::UnixStream;

    let stream = UnixStream::connect(watch_socket).await?;
    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        if let Err(e) = conn.await {
            tracing::warn!(error = %e, "watch connection error during rollback");
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri(format!("/switch/rollback/{}", switch_id))
        .body(Full::new(Bytes::new()))?;

    let _resp = sender.send_request(req).await?;
    Ok(())
}

/// Execute an optimization action.
async fn execute_action(action: &OptAction) -> Result<()> {
    match action {
        OptAction::ServiceRestart { name } => {
            let output = tokio::process::Command::new("systemctl")
                .args(["restart", name])
                .output()
                .await
                .context("failed to run systemctl restart")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("systemctl restart {} failed: {}", name, stderr);
            }
            tracing::info!(service = %name, "optimization: restarted service");
            Ok(())
        }
        OptAction::Sysctl { key, value } => {
            let output = tokio::process::Command::new("sysctl")
                .args(["-w", &format!("{}={}", key, value)])
                .output()
                .await
                .context("failed to run sysctl")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("sysctl -w {}={} failed: {}", key, value, stderr);
            }
            tracing::info!(key = %key, value = %value, "optimization: applied sysctl");
            Ok(())
        }
        OptAction::NixConfig { diff } => {
            tracing::info!(diff_len = diff.len(), "optimization: NixConfig change (requires manual apply)");
            // NixOS config changes are complex — log but don't auto-apply
            Ok(())
        }
        OptAction::Custom { command } => {
            let output = tokio::process::Command::new("sh")
                .args(["-c", command])
                .output()
                .await
                .context("failed to run custom command")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                bail!("custom command failed: {}", stderr);
            }
            tracing::info!(command = %command, "optimization: executed custom command");
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{init_db, insert_knowledge_doc, KnowledgeDoc};
    use chrono::Utc;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    #[test]
    fn test_suggest_optimizations_for_recurring_failure() {
        let conn = setup_db();
        let now = Utc::now();

        insert_knowledge_doc(
            &conn,
            &KnowledgeDoc {
                id: "kd-1".to_string(),
                title: "nginx.service recurring failures".to_string(),
                category: "reliability".to_string(),
                content: "nginx keeps failing".to_string(),
                source_patterns: vec![],
                confidence: 0.9,
                created_at: now,
                updated_at: now,
                applied: false,
                tags: vec!["recurring".to_string()],
            },
        )
        .unwrap();

        let suggestions = suggest_optimizations(&conn).unwrap();
        assert_eq!(suggestions.len(), 1);
        assert!(suggestions[0].description.contains("nginx"));
        assert!(matches!(
            suggestions[0].action,
            OptAction::ServiceRestart { .. }
        ));
    }

    #[test]
    fn test_approve_optimization() {
        let conn = setup_db();
        let opt = Optimization {
            id: "opt-1".to_string(),
            knowledge_doc_id: "kd-1".to_string(),
            description: "restart nginx".to_string(),
            action: OptAction::ServiceRestart {
                name: "nginx".to_string(),
            },
            status: OptStatus::Suggested,
            switch_id: None,
            created_at: Utc::now(),
        };
        insert_optimization(&conn, &opt).unwrap();

        let approved = approve_optimization(&conn, "opt-1").unwrap();
        assert_eq!(approved.status, OptStatus::Approved);
    }
}
