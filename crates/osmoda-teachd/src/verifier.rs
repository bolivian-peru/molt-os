use std::sync::Arc;

use chrono::Utc;
use serde::Serialize;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::knowledge::{
    insert_skill_candidate, list_skill_candidates, skill_success_rate, SkillCandidateStatus,
};
use crate::TeachdState;

// ── Thresholds ──

const MIN_EXECUTIONS: u32 = 5;
const PROMOTE_RATE: f64 = 0.80;
const REJECT_RATE: f64 = 0.30;
const RETIRE_RATE: f64 = 0.40;
const CONFIDENCE_EXEC_WEIGHT: f64 = 0.4;
const VERIFY_INTERVAL_SECS: u64 = 1800;
const VERIFY_INITIAL_DELAY_SECS: u64 = 120;

// ── Result type ──

#[derive(Debug, Clone, Serialize, Default)]
pub struct VerifyCycleResult {
    pub promoted: Vec<String>,
    pub rejected: Vec<String>,
    pub retired: Vec<String>,
    pub updated: Vec<String>,
    pub skipped: u32,
}

// ── Background loop ──

/// Background loop: evaluates skill execution outcomes every 30 minutes.
pub async fn verifier_loop(state: Arc<Mutex<TeachdState>>, cancel: CancellationToken) {
    // Initial delay: let other loops settle
    tokio::select! {
        _ = cancel.cancelled() => return,
        _ = tokio::time::sleep(tokio::time::Duration::from_secs(VERIFY_INITIAL_DELAY_SECS)) => {}
    }

    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(VERIFY_INTERVAL_SECS));

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                tracing::info!("verifier loop shutting down");
                break;
            }
            _ = interval.tick() => {
                if let Err(e) = run_verify_cycle(&state).await {
                    tracing::warn!(error = %e, "verify cycle failed");
                }
            }
        }
    }
}

// ── Cycle logic (also called by POST /skills/verify) ──

pub async fn run_verify_cycle(
    state: &Arc<Mutex<TeachdState>>,
) -> anyhow::Result<VerifyCycleResult> {
    let st = state.lock().await;

    let generated = list_skill_candidates(&st.db, Some("generated"), 100)?;
    let promoted = list_skill_candidates(&st.db, Some("promoted"), 100)?;
    let all_candidates: Vec<_> = generated.into_iter().chain(promoted).collect();

    if all_candidates.is_empty() {
        return Ok(VerifyCycleResult::default());
    }

    let mut result = VerifyCycleResult::default();

    for candidate in &all_candidates {
        let (success, total) = skill_success_rate(&st.db, &candidate.name)?;

        if total < MIN_EXECUTIONS {
            result.skipped += 1;
            continue;
        }

        let rate = success as f64 / total as f64;
        let mut updated = candidate.clone();

        // Blend confidence: initial weight + execution-derived weight
        let blended = candidate.confidence * (1.0 - CONFIDENCE_EXEC_WEIGHT)
            + rate * CONFIDENCE_EXEC_WEIGHT;
        updated.confidence = blended.clamp(0.0, 1.0);
        updated.updated_at = Utc::now();

        let action: Option<&str>;

        match candidate.status {
            SkillCandidateStatus::Generated => {
                if rate >= PROMOTE_RATE {
                    updated.status = SkillCandidateStatus::Promoted;
                    if let Some(ref path) = updated.skill_path {
                        promote_skill_file(path);
                    }
                    action = Some("auto-promoted");
                    result.promoted.push(candidate.name.clone());
                } else if rate < REJECT_RATE {
                    updated.status = SkillCandidateStatus::Rejected;
                    action = Some("auto-rejected");
                    result.rejected.push(candidate.name.clone());
                } else {
                    action = None;
                    result.updated.push(candidate.name.clone());
                }
            }
            SkillCandidateStatus::Promoted => {
                if rate < RETIRE_RATE {
                    updated.status = SkillCandidateStatus::Retired;
                    if let Some(ref path) = updated.skill_path {
                        retire_skill_file(path);
                    }
                    action = Some("retired");
                    result.retired.push(candidate.name.clone());
                } else {
                    action = None;
                    result.updated.push(candidate.name.clone());
                }
            }
            _ => continue,
        }

        insert_skill_candidate(&st.db, &updated)?;

        if let Some(act) = action {
            st.receipt_logger
                .log_event(
                    &format!("skill.{}", act),
                    &candidate.name,
                    &format!(
                        "rate={:.0}% ({}/{}), confidence={:.2}->{:.2}",
                        rate * 100.0,
                        success,
                        total,
                        candidate.confidence,
                        updated.confidence,
                    ),
                )
                .await;
        }
    }

    if !result.promoted.is_empty() || !result.rejected.is_empty() || !result.retired.is_empty() {
        tracing::info!(
            promoted = result.promoted.len(),
            rejected = result.rejected.len(),
            retired = result.retired.len(),
            updated = result.updated.len(),
            skipped = result.skipped,
            "verify cycle complete"
        );
    }

    Ok(result)
}

// ── File helpers ──

fn promote_skill_file(path: &str) {
    if let Ok(content) = std::fs::read_to_string(path) {
        let updated = content.replace("activation: manual", "activation: auto");
        if let Err(e) = std::fs::write(path, &updated) {
            tracing::warn!(error = %e, path = %path, "failed to update SKILL.md for promotion");
        }
    }
}

fn retire_skill_file(path: &str) {
    if let Ok(content) = std::fs::read_to_string(path) {
        let updated = content.replace("activation: auto", "activation: retired");
        if let Err(e) = std::fs::write(path, &updated) {
            tracing::warn!(error = %e, path = %path, "failed to update SKILL.md for retirement");
        }
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge::{
        init_db, insert_skill_candidate, insert_skill_execution, SkillCandidate,
        SkillCandidateStatus, SkillExecution,
    };
    use crate::receipt::ReceiptLogger;
    use chrono::Utc;
    use rusqlite::Connection;

    fn setup_state() -> Arc<Mutex<TeachdState>> {
        let db = Connection::open_in_memory().unwrap();
        init_db(&db).unwrap();
        Arc::new(Mutex::new(TeachdState {
            db,
            state_dir: "/tmp/test-teachd".to_string(),
            skills_dir: "/tmp/test-teachd/skills".to_string(),
            agentd_socket: "/tmp/test-agentd.sock".to_string(),
            watch_socket: "/tmp/test-watch.sock".to_string(),
            receipt_logger: ReceiptLogger::new("/tmp/test-agentd.sock"),
        }))
    }

    fn make_candidate(name: &str, status: SkillCandidateStatus, confidence: f64) -> SkillCandidate {
        let now = Utc::now();
        SkillCandidate {
            id: format!("sc-{}", name),
            name: name.to_string(),
            description: "test skill".to_string(),
            tools: vec!["tool_a".to_string(), "tool_b".to_string()],
            session_count: 5,
            confidence,
            source_patterns: Vec::new(),
            status,
            skill_path: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn add_executions(db: &Connection, skill_name: &str, successes: u32, failures: u32) {
        let now = Utc::now();
        for i in 0..successes {
            insert_skill_execution(
                db,
                &SkillExecution {
                    id: format!("se-{}-s{}", skill_name, i),
                    skill_name: skill_name.to_string(),
                    session_id: "test-session".to_string(),
                    ts: now,
                    outcome: "success".to_string(),
                    notes: None,
                },
            )
            .unwrap();
        }
        for i in 0..failures {
            insert_skill_execution(
                db,
                &SkillExecution {
                    id: format!("se-{}-f{}", skill_name, i),
                    skill_name: skill_name.to_string(),
                    session_id: "test-session".to_string(),
                    ts: now,
                    outcome: "failure".to_string(),
                    notes: None,
                },
            )
            .unwrap();
        }
    }

    #[tokio::test]
    async fn test_verify_promotes_successful_skill() {
        let state = setup_state();
        {
            let st = state.lock().await;
            let c = make_candidate("check-restart", SkillCandidateStatus::Generated, 0.7);
            insert_skill_candidate(&st.db, &c).unwrap();
            add_executions(&st.db, "check-restart", 5, 1); // 83% success
        }

        let result = run_verify_cycle(&state).await.unwrap();
        assert_eq!(result.promoted, vec!["check-restart"]);
        assert!(result.rejected.is_empty());

        let st = state.lock().await;
        let c = crate::knowledge::get_skill_candidate(&st.db, "sc-check-restart")
            .unwrap()
            .unwrap();
        assert_eq!(c.status, SkillCandidateStatus::Promoted);
    }

    #[tokio::test]
    async fn test_verify_rejects_failing_skill() {
        let state = setup_state();
        {
            let st = state.lock().await;
            let c = make_candidate("bad-skill", SkillCandidateStatus::Generated, 0.5);
            insert_skill_candidate(&st.db, &c).unwrap();
            add_executions(&st.db, "bad-skill", 1, 5); // 17% success
        }

        let result = run_verify_cycle(&state).await.unwrap();
        assert_eq!(result.rejected, vec!["bad-skill"]);

        let st = state.lock().await;
        let c = crate::knowledge::get_skill_candidate(&st.db, "sc-bad-skill")
            .unwrap()
            .unwrap();
        assert_eq!(c.status, SkillCandidateStatus::Rejected);
    }

    #[tokio::test]
    async fn test_verify_retires_degraded_skill() {
        let state = setup_state();
        {
            let st = state.lock().await;
            let c = make_candidate("degraded", SkillCandidateStatus::Promoted, 0.9);
            insert_skill_candidate(&st.db, &c).unwrap();
            add_executions(&st.db, "degraded", 1, 5); // 17% success
        }

        let result = run_verify_cycle(&state).await.unwrap();
        assert_eq!(result.retired, vec!["degraded"]);

        let st = state.lock().await;
        let c = crate::knowledge::get_skill_candidate(&st.db, "sc-degraded")
            .unwrap()
            .unwrap();
        assert_eq!(c.status, SkillCandidateStatus::Retired);
    }

    #[tokio::test]
    async fn test_verify_skips_insufficient_data() {
        let state = setup_state();
        {
            let st = state.lock().await;
            let c = make_candidate("new-skill", SkillCandidateStatus::Generated, 0.6);
            insert_skill_candidate(&st.db, &c).unwrap();
            add_executions(&st.db, "new-skill", 2, 0); // only 2 runs
        }

        let result = run_verify_cycle(&state).await.unwrap();
        assert_eq!(result.skipped, 1);
        assert!(result.promoted.is_empty());
        assert!(result.rejected.is_empty());

        let st = state.lock().await;
        let c = crate::knowledge::get_skill_candidate(&st.db, "sc-new-skill")
            .unwrap()
            .unwrap();
        assert_eq!(c.status, SkillCandidateStatus::Generated); // unchanged
    }

    #[tokio::test]
    async fn test_verify_hold_zone() {
        let state = setup_state();
        {
            let st = state.lock().await;
            let c = make_candidate("middling", SkillCandidateStatus::Generated, 0.7);
            insert_skill_candidate(&st.db, &c).unwrap();
            add_executions(&st.db, "middling", 3, 3); // 50% success — hold zone
        }

        let result = run_verify_cycle(&state).await.unwrap();
        assert!(result.promoted.is_empty());
        assert!(result.rejected.is_empty());
        assert_eq!(result.updated, vec!["middling"]);

        let st = state.lock().await;
        let c = crate::knowledge::get_skill_candidate(&st.db, "sc-middling")
            .unwrap()
            .unwrap();
        assert_eq!(c.status, SkillCandidateStatus::Generated); // still generated
        // Confidence should be blended: 0.7 * 0.6 + 0.5 * 0.4 = 0.62
        assert!((c.confidence - 0.62).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_confidence_blending() {
        let state = setup_state();
        {
            let st = state.lock().await;
            let c = make_candidate("blend-test", SkillCandidateStatus::Generated, 0.9);
            insert_skill_candidate(&st.db, &c).unwrap();
            add_executions(&st.db, "blend-test", 3, 3); // 50% success
        }

        let result = run_verify_cycle(&state).await.unwrap();
        assert_eq!(result.updated, vec!["blend-test"]);

        let st = state.lock().await;
        let c = crate::knowledge::get_skill_candidate(&st.db, "sc-blend-test")
            .unwrap()
            .unwrap();
        // 0.9 * 0.6 + 0.5 * 0.4 = 0.54 + 0.20 = 0.74
        assert!((c.confidence - 0.74).abs() < 0.01);
    }
}
