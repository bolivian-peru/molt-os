use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::switch::HealthCheck;

/// Fleet-wide SafeSwitch coordination phases.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FleetPhase {
    Propose,
    Vote,
    Execute,
    Verify,
    Committed,
    RolledBack { reason: String },
    Aborted { reason: String },
}

/// A vote from a fleet participant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetVote {
    pub peer_id: String,
    pub approve: bool,
    pub reason: Option<String>,
    pub voted_at: String,
}

/// Status of a participant in a fleet switch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ParticipantStatus {
    Pending,
    Voted,
    Executing,
    Healthy,
    Failed { reason: String },
}

/// A participant in a fleet switch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetParticipant {
    pub peer_id: String,
    pub status: ParticipantStatus,
    pub local_switch_id: Option<String>,
}

/// A fleet-wide SafeSwitch session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetSwitch {
    pub id: String,
    pub plan: String,
    pub proposer: String,
    pub participants: Vec<FleetParticipant>,
    pub votes: Vec<FleetVote>,
    pub quorum_percent: u8,
    pub phase: FleetPhase,
    pub health_checks: Vec<HealthCheck>,
    pub created_at: String,
    pub timeout_secs: u64,
    pub result_summary: Option<String>,
}

/// Default fleet timeout: 5 minutes.
const DEFAULT_FLEET_TIMEOUT_SECS: u64 = 300;

/// Default quorum: >50%.
const DEFAULT_QUORUM_PERCENT: u8 = 51;

impl FleetSwitch {
    /// Create a new fleet switch proposal.
    pub fn new(
        plan: &str,
        proposer: &str,
        peer_ids: Vec<String>,
        health_checks: Vec<HealthCheck>,
        quorum_percent: Option<u8>,
        timeout_secs: Option<u64>,
    ) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let participants = peer_ids
            .into_iter()
            .map(|pid| FleetParticipant {
                peer_id: pid,
                status: ParticipantStatus::Pending,
                local_switch_id: None,
            })
            .collect();

        Self {
            id,
            plan: plan.to_string(),
            proposer: proposer.to_string(),
            participants,
            votes: Vec::new(),
            quorum_percent: quorum_percent.unwrap_or(DEFAULT_QUORUM_PERCENT),
            phase: FleetPhase::Propose,
            health_checks,
            created_at: chrono::Utc::now().to_rfc3339(),
            timeout_secs: timeout_secs.unwrap_or(DEFAULT_FLEET_TIMEOUT_SECS),
            result_summary: None,
        }
    }

    /// Total number of participants (including proposer).
    pub fn participant_count(&self) -> usize {
        self.participants.len()
    }

    /// Number of votes required for quorum.
    pub fn quorum_required(&self) -> usize {
        let total = self.participant_count();
        let required = (total as f64 * self.quorum_percent as f64 / 100.0).ceil() as usize;
        required.max(1)
    }

    /// Number of approve votes received.
    pub fn approve_count(&self) -> usize {
        self.votes.iter().filter(|v| v.approve).count()
    }

    /// Number of deny votes received.
    pub fn deny_count(&self) -> usize {
        self.votes.iter().filter(|v| !v.approve).count()
    }

    /// Check if quorum has been reached.
    pub fn has_quorum(&self) -> bool {
        self.approve_count() >= self.quorum_required()
    }

    /// Check if the proposal is vetoed (enough denies to make quorum impossible).
    pub fn is_vetoed(&self) -> bool {
        let remaining = self.participant_count() - self.votes.len();
        let max_possible_approves = self.approve_count() + remaining;
        max_possible_approves < self.quorum_required()
    }

    /// Check if the fleet switch has timed out.
    pub fn is_timed_out(&self) -> bool {
        if let Ok(created) = chrono::DateTime::parse_from_rfc3339(&self.created_at) {
            let elapsed = chrono::Utc::now().signed_duration_since(created);
            elapsed.num_seconds() as u64 >= self.timeout_secs
        } else {
            false
        }
    }

    /// Record a vote from a participant.
    pub fn record_vote(&mut self, peer_id: &str, approve: bool, reason: Option<String>) -> bool {
        // Check for duplicate vote
        if self.votes.iter().any(|v| v.peer_id == peer_id) {
            return false;
        }

        // Check if peer is a participant
        if !self.participants.iter().any(|p| p.peer_id == peer_id) {
            return false;
        }

        self.votes.push(FleetVote {
            peer_id: peer_id.to_string(),
            approve,
            reason,
            voted_at: chrono::Utc::now().to_rfc3339(),
        });

        // Update participant status
        if let Some(p) = self.participants.iter_mut().find(|p| p.peer_id == peer_id) {
            p.status = ParticipantStatus::Voted;
        }

        true
    }

    /// Advance phase to Execute (after quorum).
    pub fn advance_to_execute(&mut self) {
        if self.has_quorum() {
            self.phase = FleetPhase::Execute;
            for p in &mut self.participants {
                if self.votes.iter().any(|v| v.peer_id == p.peer_id && v.approve) {
                    p.status = ParticipantStatus::Executing;
                }
            }
        }
    }

    /// Record health check results from a participant.
    pub fn record_health_result(&mut self, peer_id: &str, healthy: bool, reason: Option<String>) {
        if let Some(p) = self.participants.iter_mut().find(|p| p.peer_id == peer_id) {
            if healthy {
                p.status = ParticipantStatus::Healthy;
            } else {
                p.status = ParticipantStatus::Failed {
                    reason: reason.unwrap_or_else(|| "health check failed".to_string()),
                };
            }
        }
    }

    /// Check if all executing participants are healthy.
    pub fn all_healthy(&self) -> bool {
        self.participants
            .iter()
            .filter(|p| matches!(p.status, ParticipantStatus::Executing | ParticipantStatus::Healthy))
            .all(|p| matches!(p.status, ParticipantStatus::Healthy))
    }

    /// Check if any executing participant failed.
    pub fn any_failed(&self) -> bool {
        self.participants
            .iter()
            .any(|p| matches!(p.status, ParticipantStatus::Failed { .. }))
    }

    /// Commit the fleet switch.
    pub fn commit(&mut self) {
        self.phase = FleetPhase::Committed;
        self.result_summary = Some(format!(
            "committed: {}/{} participants healthy",
            self.participants.iter().filter(|p| matches!(p.status, ParticipantStatus::Healthy)).count(),
            self.participant_count()
        ));
    }

    /// Rollback the fleet switch.
    pub fn rollback(&mut self, reason: &str) {
        self.phase = FleetPhase::RolledBack {
            reason: reason.to_string(),
        };
        self.result_summary = Some(format!("rolled back: {reason}"));
    }

    /// Abort the fleet switch.
    pub fn abort(&mut self, reason: &str) {
        self.phase = FleetPhase::Aborted {
            reason: reason.to_string(),
        };
        self.result_summary = Some(format!("aborted: {reason}"));
    }
}

/// Fleet coordinator that manages fleet-wide switches.
pub struct FleetCoordinator {
    pub switches: HashMap<String, FleetSwitch>,
}

impl FleetCoordinator {
    pub fn new() -> Self {
        Self {
            switches: HashMap::new(),
        }
    }

    /// Create a new fleet switch proposal.
    pub fn propose(
        &mut self,
        plan: &str,
        proposer: &str,
        peer_ids: Vec<String>,
        health_checks: Vec<HealthCheck>,
        quorum_percent: Option<u8>,
        timeout_secs: Option<u64>,
    ) -> FleetSwitch {
        let switch = FleetSwitch::new(plan, proposer, peer_ids, health_checks, quorum_percent, timeout_secs);
        let result = switch.clone();
        self.switches.insert(switch.id.clone(), switch);
        result
    }

    /// Get a fleet switch by ID.
    pub fn get(&self, id: &str) -> Option<&FleetSwitch> {
        self.switches.get(id)
    }

    /// Get a mutable fleet switch by ID.
    pub fn get_mut(&mut self, id: &str) -> Option<&mut FleetSwitch> {
        self.switches.get_mut(id)
    }

    /// List all fleet switches.
    pub fn list(&self) -> Vec<&FleetSwitch> {
        self.switches.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fleet_switch_creation() {
        let fs = FleetSwitch::new(
            "upgrade nginx",
            "proposer-1",
            vec!["peer-a".to_string(), "peer-b".to_string(), "peer-c".to_string()],
            vec![],
            None,
            None,
        );

        assert_eq!(fs.participant_count(), 3);
        assert_eq!(fs.quorum_required(), 2); // ceil(3 * 0.51) = 2
        assert_eq!(fs.phase, FleetPhase::Propose);
    }

    #[test]
    fn test_quorum_calculation() {
        let fs = FleetSwitch::new(
            "test",
            "p",
            vec!["a".into(), "b".into()],
            vec![],
            Some(51),
            None,
        );
        assert_eq!(fs.quorum_required(), 2); // ceil(2 * 0.51) = 2

        let fs2 = FleetSwitch::new(
            "test",
            "p",
            vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()],
            vec![],
            Some(60),
            None,
        );
        assert_eq!(fs2.quorum_required(), 3); // ceil(5 * 0.60) = 3
    }

    #[test]
    fn test_voting() {
        let mut fs = FleetSwitch::new(
            "test",
            "p",
            vec!["a".into(), "b".into(), "c".into()],
            vec![],
            None,
            None,
        );

        assert!(!fs.has_quorum());

        // Vote from peer-a (approve)
        assert!(fs.record_vote("a", true, None));
        assert!(!fs.has_quorum());

        // Vote from peer-b (approve)
        assert!(fs.record_vote("b", true, None));
        assert!(fs.has_quorum());

        // Duplicate vote should fail
        assert!(!fs.record_vote("a", true, None));

        // Non-participant vote should fail
        assert!(!fs.record_vote("unknown", true, None));
    }

    #[test]
    fn test_veto() {
        let mut fs = FleetSwitch::new(
            "test",
            "p",
            vec!["a".into(), "b".into(), "c".into()],
            vec![],
            None,
            None,
        );

        // Two denies when quorum needs 2 approves out of 3
        fs.record_vote("a", false, None);
        fs.record_vote("b", false, None);

        // With 2 denies and only 1 remaining, max possible approves = 0 + 1 = 1 < 2
        assert!(fs.is_vetoed());
    }

    #[test]
    fn test_advance_to_execute() {
        let mut fs = FleetSwitch::new(
            "test",
            "p",
            vec!["a".into(), "b".into()],
            vec![],
            Some(51),
            None,
        );

        fs.record_vote("a", true, None);
        fs.record_vote("b", true, None);
        fs.advance_to_execute();

        assert_eq!(fs.phase, FleetPhase::Execute);
    }

    #[test]
    fn test_health_results() {
        let mut fs = FleetSwitch::new(
            "test",
            "p",
            vec!["a".into(), "b".into()],
            vec![],
            None,
            None,
        );

        fs.record_vote("a", true, None);
        fs.record_vote("b", true, None);
        fs.advance_to_execute();

        fs.record_health_result("a", true, None);
        assert!(!fs.all_healthy()); // b still executing

        fs.record_health_result("b", true, None);
        assert!(fs.all_healthy());
        assert!(!fs.any_failed());
    }

    #[test]
    fn test_health_failure_triggers_rollback_path() {
        let mut fs = FleetSwitch::new(
            "test",
            "p",
            vec!["a".into(), "b".into()],
            vec![],
            None,
            None,
        );

        fs.record_vote("a", true, None);
        fs.record_vote("b", true, None);
        fs.advance_to_execute();

        fs.record_health_result("a", true, None);
        fs.record_health_result("b", false, Some("nginx down".to_string()));

        assert!(fs.any_failed());
        fs.rollback("participant b failed health check");

        match &fs.phase {
            FleetPhase::RolledBack { reason } => {
                assert!(reason.contains("participant b"));
            }
            _ => panic!("expected RolledBack phase"),
        }
    }

    #[test]
    fn test_timeout() {
        let mut fs = FleetSwitch::new(
            "test",
            "p",
            vec!["a".into()],
            vec![],
            None,
            Some(0), // immediate timeout
        );

        // Allow a tiny bit of time to pass
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(fs.is_timed_out());

        fs.abort("timeout");
        match &fs.phase {
            FleetPhase::Aborted { reason } => assert!(reason.contains("timeout")),
            _ => panic!("expected Aborted phase"),
        }
    }

    #[test]
    fn test_coordinator_lifecycle() {
        let mut coord = FleetCoordinator::new();

        let fs = coord.propose(
            "upgrade plan",
            "me",
            vec!["a".into(), "b".into()],
            vec![],
            None,
            None,
        );

        assert!(coord.get(&fs.id).is_some());
        assert_eq!(coord.list().len(), 1);

        // Vote
        let s = coord.get_mut(&fs.id).unwrap();
        s.record_vote("a", true, None);
        s.record_vote("b", true, None);
        assert!(s.has_quorum());
    }
}
