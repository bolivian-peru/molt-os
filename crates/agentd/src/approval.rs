use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// Commands that are always considered destructive, regardless of configuration.
const DANGEROUS_COMMANDS: &[&str] = &[
    "rm -rf",
    "mkfs",
    "dd if=",
    "wipefs",
    "fdisk",
    "parted",
    "sgdisk",
    "shred",
    "> /dev/sd",
    "nix-collect-garbage",
    "nixos-rebuild",
    "systemctl disable",
    "systemctl mask",
    "systemctl stop",
    "userdel",
    "groupdel",
    "passwd",
    "chown -R",
    "chmod -R",
    "iptables -F",
    "nft flush",
    "reboot",
    "shutdown",
    "poweroff",
    "halt",
    "kill -9",
    "pkill",
    "killall",
];

/// Operations that require approval (matches NixOS approvalRequired list).
const DANGEROUS_OPERATIONS: &[&str] = &[
    "nix.rebuild",
    "system.user.create",
    "system.user.delete",
    "system.firewall.modify",
    "system.disk.format",
    "system.reboot",
    "system.shutdown",
    "wallet.send",
    "wallet.create",
    "switch.begin",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

impl std::fmt::Display for ApprovalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApprovalStatus::Pending => write!(f, "pending"),
            ApprovalStatus::Approved => write!(f, "approved"),
            ApprovalStatus::Denied => write!(f, "denied"),
            ApprovalStatus::Expired => write!(f, "expired"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    pub id: String,
    pub command: String,
    pub actor: String,
    pub reason: String,
    pub created_at: String,
    pub expires_at: String,
    pub status: ApprovalStatus,
    pub decided_at: Option<String>,
    pub decided_by: Option<String>,
}

/// Default approval TTL: 10 minutes.
const DEFAULT_TTL_SECS: i64 = 600;

/// How often the expiry loop checks for expired approvals (seconds).
pub const EXPIRY_CHECK_INTERVAL_SECS: u64 = 30;

pub struct ApprovalGate {
    /// Shared SQLite connection (thread-safe via std::sync::Mutex).
    conn: std::sync::Mutex<Connection>,
    /// Additional patterns from NixOS config that are considered destructive.
    extra_patterns: Vec<String>,
}

impl ApprovalGate {
    /// Create a new ApprovalGate, initializing the pending_approvals table.
    pub fn new(db_path: &str, extra_patterns: Vec<String>) -> Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("failed to open approval DB at {db_path}"))?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending_approvals (
                id TEXT PRIMARY KEY,
                command TEXT NOT NULL,
                actor TEXT NOT NULL,
                reason TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                expires_at TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                decided_at TEXT,
                decided_by TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_approval_status ON pending_approvals(status);",
        )
        .context("failed to create pending_approvals table")?;

        Ok(Self {
            conn: std::sync::Mutex::new(conn),
            extra_patterns,
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("approval DB lock poisoned")
    }

    /// Check whether a command/operation is destructive and requires approval.
    pub fn is_destructive(&self, command: &str) -> bool {
        let lower = command.to_lowercase();

        // Check built-in dangerous commands
        for pattern in DANGEROUS_COMMANDS {
            if lower.contains(pattern) {
                return true;
            }
        }

        // Check dangerous operations
        for op in DANGEROUS_OPERATIONS {
            if lower == *op || lower.starts_with(&format!("{op}.")) {
                return true;
            }
        }

        // Check extra patterns from NixOS config
        for pattern in &self.extra_patterns {
            let p = pattern.to_lowercase();
            if lower.contains(&p) || lower == p {
                return true;
            }
        }

        false
    }

    /// Request approval for a destructive operation. Returns the approval ID.
    pub fn request_approval(
        &self,
        command: &str,
        actor: &str,
        reason: &str,
        ttl_secs: Option<i64>,
    ) -> Result<PendingApproval> {
        let conn = self.conn();
        let id = uuid::Uuid::new_v4().to_string();
        let ttl = ttl_secs.unwrap_or(DEFAULT_TTL_SECS);
        let now = chrono::Utc::now();
        let created_at = now.to_rfc3339();
        let expires_at = (now + chrono::Duration::seconds(ttl)).to_rfc3339();

        conn.execute(
            "INSERT INTO pending_approvals (id, command, actor, reason, created_at, expires_at, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending')",
            params![id, command, actor, reason, created_at, expires_at],
        )
        .context("failed to insert pending approval")?;

        Ok(PendingApproval {
            id,
            command: command.to_string(),
            actor: actor.to_string(),
            reason: reason.to_string(),
            created_at,
            expires_at,
            status: ApprovalStatus::Pending,
            decided_at: None,
            decided_by: None,
        })
    }

    /// Check the status of an approval request.
    pub fn check_approval(&self, id: &str) -> Result<Option<PendingApproval>> {
        let conn = self.conn();
        let result = conn.query_row(
            "SELECT id, command, actor, reason, created_at, expires_at, status, decided_at, decided_by
             FROM pending_approvals WHERE id = ?1",
            params![id],
            |row| {
                Ok(PendingApproval {
                    id: row.get(0)?,
                    command: row.get(1)?,
                    actor: row.get(2)?,
                    reason: row.get(3)?,
                    created_at: row.get(4)?,
                    expires_at: row.get(5)?,
                    status: parse_status(&row.get::<_, String>(6)?),
                    decided_at: row.get(7)?,
                    decided_by: row.get(8)?,
                })
            },
        );

        match result {
            Ok(approval) => Ok(Some(approval)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Approve a pending request.
    pub fn approve(&self, id: &str, decided_by: &str) -> Result<PendingApproval> {
        let conn = self.conn();
        let now = chrono::Utc::now().to_rfc3339();

        let rows = conn.execute(
            "UPDATE pending_approvals SET status = 'approved', decided_at = ?1, decided_by = ?2
             WHERE id = ?3 AND status = 'pending'",
            params![now, decided_by, id],
        )?;

        if rows == 0 {
            anyhow::bail!("approval {id} not found or not in pending state");
        }

        // Inline the query to avoid deadlock (conn lock already held)
        let result = conn.query_row(
            "SELECT id, command, actor, reason, created_at, expires_at, status, decided_at, decided_by
             FROM pending_approvals WHERE id = ?1",
            params![id],
            |row| {
                Ok(PendingApproval {
                    id: row.get(0)?,
                    command: row.get(1)?,
                    actor: row.get(2)?,
                    reason: row.get(3)?,
                    created_at: row.get(4)?,
                    expires_at: row.get(5)?,
                    status: parse_status(&row.get::<_, String>(6)?),
                    decided_at: row.get(7)?,
                    decided_by: row.get(8)?,
                })
            },
        )?;

        Ok(result)
    }

    /// Deny a pending request.
    pub fn deny(&self, id: &str, decided_by: &str) -> Result<PendingApproval> {
        let conn = self.conn();
        let now = chrono::Utc::now().to_rfc3339();

        let rows = conn.execute(
            "UPDATE pending_approvals SET status = 'denied', decided_at = ?1, decided_by = ?2
             WHERE id = ?3 AND status = 'pending'",
            params![now, decided_by, id],
        )?;

        if rows == 0 {
            anyhow::bail!("approval {id} not found or not in pending state");
        }

        // Inline the query to avoid deadlock (conn lock already held)
        let result = conn.query_row(
            "SELECT id, command, actor, reason, created_at, expires_at, status, decided_at, decided_by
             FROM pending_approvals WHERE id = ?1",
            params![id],
            |row| {
                Ok(PendingApproval {
                    id: row.get(0)?,
                    command: row.get(1)?,
                    actor: row.get(2)?,
                    reason: row.get(3)?,
                    created_at: row.get(4)?,
                    expires_at: row.get(5)?,
                    status: parse_status(&row.get::<_, String>(6)?),
                    decided_at: row.get(7)?,
                    decided_by: row.get(8)?,
                })
            },
        )?;

        Ok(result)
    }

    /// List pending approvals.
    pub fn list_pending(&self) -> Result<Vec<PendingApproval>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, command, actor, reason, created_at, expires_at, status, decided_at, decided_by
             FROM pending_approvals WHERE status = 'pending'
             ORDER BY created_at DESC",
        )?;

        let approvals = stmt
            .query_map([], |row| {
                Ok(PendingApproval {
                    id: row.get(0)?,
                    command: row.get(1)?,
                    actor: row.get(2)?,
                    reason: row.get(3)?,
                    created_at: row.get(4)?,
                    expires_at: row.get(5)?,
                    status: parse_status(&row.get::<_, String>(6)?),
                    decided_at: row.get(7)?,
                    decided_by: row.get(8)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list pending approvals")?;

        Ok(approvals)
    }

    /// Expire all pending approvals that have passed their expiry time.
    /// Returns the number of expired approvals.
    pub fn expire_stale(&self) -> Result<usize> {
        let conn = self.conn();
        let now = chrono::Utc::now().to_rfc3339();

        let rows = conn.execute(
            "UPDATE pending_approvals SET status = 'expired'
             WHERE status = 'pending' AND expires_at < ?1",
            params![now],
        )?;

        if rows > 0 {
            tracing::info!(count = rows, "expired stale approval requests");
        }

        Ok(rows)
    }
}

fn parse_status(s: &str) -> ApprovalStatus {
    match s {
        "approved" => ApprovalStatus::Approved,
        "denied" => ApprovalStatus::Denied,
        "expired" => ApprovalStatus::Expired,
        _ => ApprovalStatus::Pending,
    }
}

/// Background task that periodically expires stale approvals.
pub async fn expiry_loop(gate: std::sync::Arc<ApprovalGate>) {
    let mut interval =
        tokio::time::interval(std::time::Duration::from_secs(EXPIRY_CHECK_INTERVAL_SECS));

    loop {
        interval.tick().await;
        if let Err(e) = gate.expire_stale() {
            tracing::warn!(error = %e, "approval expiry check failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_gate() -> ApprovalGate {
        ApprovalGate::new(":memory:", vec![]).unwrap()
    }

    fn gate_with_extras(extras: Vec<String>) -> ApprovalGate {
        ApprovalGate::new(":memory:", extras).unwrap()
    }

    #[test]
    fn test_is_destructive_dangerous_commands() {
        let gate = test_gate();
        assert!(gate.is_destructive("rm -rf /"));
        assert!(gate.is_destructive("sudo rm -rf /var/lib"));
        assert!(gate.is_destructive("mkfs.ext4 /dev/sda1"));
        assert!(gate.is_destructive("dd if=/dev/zero of=/dev/sda"));
        assert!(gate.is_destructive("reboot"));
        assert!(gate.is_destructive("shutdown -h now"));
        assert!(gate.is_destructive("kill -9 1234"));
    }

    #[test]
    fn test_is_not_destructive_safe_commands() {
        let gate = test_gate();
        assert!(!gate.is_destructive("ls -la"));
        assert!(!gate.is_destructive("cat /etc/hostname"));
        assert!(!gate.is_destructive("systemctl status sshd"));
        assert!(!gate.is_destructive("ps aux"));
        assert!(!gate.is_destructive("df -h"));
        assert!(!gate.is_destructive("free -m"));
    }

    #[test]
    fn test_is_destructive_operations() {
        let gate = test_gate();
        assert!(gate.is_destructive("nix.rebuild"));
        assert!(gate.is_destructive("system.user.delete"));
        assert!(gate.is_destructive("wallet.send"));
        assert!(gate.is_destructive("switch.begin"));
    }

    #[test]
    fn test_is_destructive_extra_patterns() {
        let gate = gate_with_extras(vec!["custom.dangerous".to_string()]);
        assert!(gate.is_destructive("custom.dangerous"));
        assert!(!gate.is_destructive("custom.safe"));
    }

    #[test]
    fn test_request_and_check() {
        let gate = test_gate();
        let approval = gate
            .request_approval("rm -rf /tmp/data", "agent", "cleanup old data", None)
            .unwrap();

        assert_eq!(approval.status, ApprovalStatus::Pending);
        assert_eq!(approval.command, "rm -rf /tmp/data");
        assert_eq!(approval.actor, "agent");

        let checked = gate.check_approval(&approval.id).unwrap().unwrap();
        assert_eq!(checked.status, ApprovalStatus::Pending);
    }

    #[test]
    fn test_approve() {
        let gate = test_gate();
        let approval = gate
            .request_approval("reboot", "agent", "system update", None)
            .unwrap();

        let approved = gate.approve(&approval.id, "admin").unwrap();
        assert_eq!(approved.status, ApprovalStatus::Approved);
        assert_eq!(approved.decided_by, Some("admin".to_string()));
        assert!(approved.decided_at.is_some());
    }

    #[test]
    fn test_deny() {
        let gate = test_gate();
        let approval = gate
            .request_approval("shutdown", "agent", "maintenance", None)
            .unwrap();

        let denied = gate.deny(&approval.id, "admin").unwrap();
        assert_eq!(denied.status, ApprovalStatus::Denied);
    }

    #[test]
    fn test_list_pending() {
        let gate = test_gate();
        gate.request_approval("reboot", "agent", "reason1", None)
            .unwrap();
        gate.request_approval("shutdown", "agent", "reason2", None)
            .unwrap();

        let pending = gate.list_pending().unwrap();
        assert_eq!(pending.len(), 2);
    }

    #[test]
    fn test_approve_removes_from_pending() {
        let gate = test_gate();
        let a = gate
            .request_approval("reboot", "agent", "test", None)
            .unwrap();
        gate.approve(&a.id, "admin").unwrap();

        let pending = gate.list_pending().unwrap();
        assert_eq!(pending.len(), 0);
    }

    #[test]
    fn test_double_approve_fails() {
        let gate = test_gate();
        let a = gate
            .request_approval("reboot", "agent", "test", None)
            .unwrap();
        gate.approve(&a.id, "admin").unwrap();
        assert!(gate.approve(&a.id, "admin").is_err());
    }

    #[test]
    fn test_expire_stale() {
        let gate = test_gate();
        // Create with 0-second TTL (immediately expired)
        gate.request_approval("reboot", "agent", "test", Some(0))
            .unwrap();

        // Small sleep to ensure expiry time has passed
        std::thread::sleep(std::time::Duration::from_millis(10));

        let expired = gate.expire_stale().unwrap();
        assert_eq!(expired, 1);

        let pending = gate.list_pending().unwrap();
        assert_eq!(pending.len(), 0);
    }

    #[test]
    fn test_nonexistent_approval() {
        let gate = test_gate();
        let result = gate.check_approval("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_case_insensitive_destructive_check() {
        let gate = test_gate();
        assert!(gate.is_destructive("RM -RF /"));
        assert!(gate.is_destructive("Reboot"));
        assert!(gate.is_destructive("SHUTDOWN"));
    }
}
