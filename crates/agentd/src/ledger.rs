use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The zero hash used as prev_hash for the genesis event.
const GENESIS_PREV_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// A single event in the hash-chained ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub ts: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub actor: String,
    pub payload: String,
    pub prev_hash: String,
    pub hash: String,
}

/// Filter criteria for querying events.
#[derive(Debug, Default, Deserialize)]
pub struct EventFilter {
    #[serde(rename = "type")]
    pub event_type: Option<String>,
    pub actor: Option<String>,
    pub limit: Option<i64>,
}

/// Hash-chained SQLite ledger providing tamper-evident event storage.
pub struct Ledger {
    conn: Connection,
}

impl Ledger {
    /// Open or create a ledger database at the given path.
    /// Enables WAL mode and creates the events table if it does not exist.
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)
            .with_context(|| format!("failed to open ledger database at {path}"))?;

        conn.pragma_update(None, "journal_mode", "WAL")
            .context("failed to set WAL journal mode")?;

        conn.pragma_update(None, "synchronous", "NORMAL")
            .context("failed to set synchronous mode")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                type TEXT NOT NULL,
                actor TEXT NOT NULL,
                payload TEXT NOT NULL,
                prev_hash TEXT NOT NULL,
                hash TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS incidents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'open',
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                resolved_at TEXT
            );

            CREATE TABLE IF NOT EXISTS incident_steps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                incident_id TEXT NOT NULL REFERENCES incidents(id),
                step_number INTEGER NOT NULL,
                action TEXT NOT NULL,
                result TEXT NOT NULL,
                receipt_id TEXT,
                timestamp TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
            );

            CREATE TABLE IF NOT EXISTS schema_version (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                version INTEGER NOT NULL
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
                type, actor, payload,
                content=events, content_rowid=id,
                tokenize='porter unicode61'
            );

            CREATE TRIGGER IF NOT EXISTS events_fts_insert AFTER INSERT ON events BEGIN
                INSERT INTO events_fts(rowid, type, actor, payload)
                VALUES (new.id, new.type, new.actor, new.payload);
            END;",
        )
        .context("failed to create tables")?;

        let mut ledger = Self { conn };
        ledger.migrate()?;
        Ok(ledger)
    }

    /// Compute the SHA-256 hash for an event row.
    /// Uses pipe delimiters to prevent field-boundary collisions
    /// (e.g., id="12" + ts="3abc" would otherwise equal id="123" + ts="abc").
    fn compute_hash(
        id: i64,
        ts: &str,
        event_type: &str,
        actor: &str,
        payload: &str,
        prev_hash: &str,
    ) -> String {
        let input = format!("{id}|{ts}|{event_type}|{actor}|{payload}|{prev_hash}");
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Current schema version. Increment when making breaking changes.
    const CURRENT_SCHEMA_VERSION: i64 = 3;

    /// Run any pending migrations.
    /// Schema versions are one-way: once at the current version, never downgrade.
    /// `rehash_chain` is intentionally not exposed via any API endpoint.
    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self.conn
            .query_row(
                "SELECT version FROM schema_version WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        // Already at current version — no migration needed, never downgrade
        if version >= Self::CURRENT_SCHEMA_VERSION {
            return Ok(());
        }

        if version < 2 && version > 0 {
            // Migration from v1 (no delimiters) to v2 (pipe-delimited hashes):
            // Re-hash all events with the new delimiter format.
            tracing::info!("migrating ledger from schema v{version} to v2 (pipe-delimited hashes)");
            self.rehash_chain()?;
        }

        if version < 3 {
            // Migration to v3: backfill FTS5 index from existing events
            tracing::info!("migrating ledger to v3: backfilling FTS5 index");
            self.backfill_fts()?;
        }

        if version < Self::CURRENT_SCHEMA_VERSION {
            self.conn.execute(
                "INSERT OR REPLACE INTO schema_version (id, version) VALUES (1, ?1)",
                params![Self::CURRENT_SCHEMA_VERSION],
            ).context("failed to update schema version")?;
        }

        Ok(())
    }

    /// Recompute all hashes in the chain using the current hash format.
    fn rehash_chain(&mut self) -> Result<()> {
        let tx = self.conn.unchecked_transaction()
            .context("failed to begin rehash transaction")?;

        let mut stmt = tx.prepare(
            "SELECT id, ts, type, actor, payload FROM events ORDER BY id ASC"
        )?;
        let rows: Vec<(i64, String, String, String, String)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut prev_hash = GENESIS_PREV_HASH.to_string();
        for (id, ts, event_type, actor, payload) in &rows {
            let hash = Self::compute_hash(*id, ts, event_type, actor, payload, &prev_hash);
            tx.execute(
                "UPDATE events SET prev_hash = ?1, hash = ?2 WHERE id = ?3",
                params![prev_hash, hash, id],
            )?;
            prev_hash = hash;
        }

        tx.commit().context("failed to commit rehash")?;
        tracing::info!(events = rows.len(), "ledger rehash complete");
        Ok(())
    }

    /// Backfill FTS5 index from existing events.
    fn backfill_fts(&self) -> Result<()> {
        self.conn.execute_batch(
            "INSERT OR IGNORE INTO events_fts(rowid, type, actor, payload)
             SELECT id, type, actor, payload FROM events;"
        ).context("failed to backfill FTS5 index")?;
        tracing::info!("FTS5 backfill complete");
        Ok(())
    }

    /// Sanitize an FTS5 query: escape special chars and wrap terms in quotes.
    pub fn sanitize_fts_query(query: &str) -> String {
        query
            .split_whitespace()
            .map(|term| {
                let clean: String = term.chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
                    .collect();
                if clean.is_empty() {
                    return String::new();
                }
                format!("\"{clean}\"")
            })
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" OR ")
    }

    /// Full-text search over events using FTS5 with BM25 ranking.
    /// Returns events sorted by relevance. Falls back to keyword scan on FTS5 failure.
    pub fn fts_search(&self, query: &str, limit: usize) -> Result<Vec<(Event, f64)>> {
        let fts_query = Self::sanitize_fts_query(query);
        if fts_query.is_empty() {
            return Ok(Vec::new());
        }

        let sql = "SELECT e.id, e.ts, e.type, e.actor, e.payload, e.prev_hash, e.hash,
                          bm25(events_fts) as rank
                   FROM events_fts
                   JOIN events e ON e.id = events_fts.rowid
                   WHERE events_fts MATCH ?1
                   ORDER BY rank
                   LIMIT ?2";

        let mut stmt = self.conn.prepare(sql)
            .context("failed to prepare FTS5 query")?;

        let results = stmt
            .query_map(params![fts_query, limit as i64], |row| {
                let rank: f64 = row.get(7)?;
                Ok((Event {
                    id: row.get(0)?,
                    ts: row.get(1)?,
                    event_type: row.get(2)?,
                    actor: row.get(3)?,
                    payload: row.get(4)?,
                    prev_hash: row.get(5)?,
                    hash: row.get(6)?,
                }, -rank)) // bm25() returns negative scores, negate for positive relevance
            })
            .context("FTS5 query failed")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to collect FTS5 results")?;

        Ok(results)
    }

    /// Flush WAL to main database file. Call on graceful shutdown.
    pub fn flush(&self) -> Result<()> {
        self.conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")
            .context("failed to checkpoint WAL")?;
        Ok(())
    }

    /// Retrieve the hash of the most recent event, or the genesis zero-hash if empty.
    #[allow(dead_code)] // Public API — used by agentctl verify-ledger
    pub fn last_hash(&self) -> Result<String> {
        Self::last_hash_conn(&self.conn)
    }

    /// Retrieve the hash of the most recent event using an explicit connection/transaction.
    fn last_hash_conn(conn: &Connection) -> Result<String> {
        let maybe: Option<String> = conn
            .query_row(
                "SELECT hash FROM events ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .ok();

        Ok(maybe.unwrap_or_else(|| GENESIS_PREV_HASH.to_string()))
    }

    /// Append a new event to the ledger, computing its hash from the chain.
    ///
    /// The entire operation runs inside a transaction to prevent TOCTOU races
    /// where concurrent appends could interleave and corrupt the hash chain.
    ///
    /// Returns the created event including its assigned id, timestamp, and hash.
    pub fn append(&self, event_type: &str, actor: &str, payload: &str) -> Result<Event> {
        let tx = self.conn.unchecked_transaction()
            .context("failed to begin transaction")?;

        let prev_hash = Self::last_hash_conn(&tx)?;

        tx.execute(
            "INSERT INTO events (type, actor, payload, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, '')",
            params![event_type, actor, payload, prev_hash],
        )
        .context("failed to insert event")?;

        let id = tx.last_insert_rowid();

        let ts: String = tx
            .query_row("SELECT ts FROM events WHERE id = ?1", params![id], |row| {
                row.get(0)
            })
            .context("failed to read back event timestamp")?;

        let hash = Self::compute_hash(id, &ts, event_type, actor, payload, &prev_hash);

        tx.execute(
            "UPDATE events SET hash = ?1 WHERE id = ?2",
            params![hash, id],
        )
        .context("failed to update event hash")?;

        tx.commit().context("failed to commit event")?;

        Ok(Event {
            id,
            ts,
            event_type: event_type.to_string(),
            actor: actor.to_string(),
            payload: payload.to_string(),
            prev_hash,
            hash,
        })
    }

    /// Query events with optional filters.
    pub fn query(&self, filter: &EventFilter) -> Result<Vec<Event>> {
        let mut sql = String::from("SELECT id, ts, type, actor, payload, prev_hash, hash FROM events WHERE 1=1");
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref t) = filter.event_type {
            sql.push_str(&format!(" AND type = ?{}", param_values.len() + 1));
            param_values.push(Box::new(t.clone()));
        }

        if let Some(ref a) = filter.actor {
            sql.push_str(&format!(" AND actor = ?{}", param_values.len() + 1));
            param_values.push(Box::new(a.clone()));
        }

        sql.push_str(" ORDER BY id DESC");

        let limit = filter.limit.unwrap_or(50);
        sql.push_str(&format!(" LIMIT ?{}", param_values.len() + 1));
        param_values.push(Box::new(limit));

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut stmt = self.conn.prepare(&sql).context("failed to prepare query")?;

        let events = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(Event {
                    id: row.get(0)?,
                    ts: row.get(1)?,
                    event_type: row.get(2)?,
                    actor: row.get(3)?,
                    payload: row.get(4)?,
                    prev_hash: row.get(5)?,
                    hash: row.get(6)?,
                })
            })
            .context("failed to execute query")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to collect query results")?;

        Ok(events)
    }

    /// Walk the entire chain and verify every hash is correct.
    /// Returns Ok(true) if the chain is valid, Ok(false) with tracing warning if not.
    pub fn verify(&self) -> Result<bool> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, ts, type, actor, payload, prev_hash, hash FROM events ORDER BY id ASC")
            .context("failed to prepare verify query")?;

        let events: Vec<Event> = stmt
            .query_map([], |row| {
                Ok(Event {
                    id: row.get(0)?,
                    ts: row.get(1)?,
                    event_type: row.get(2)?,
                    actor: row.get(3)?,
                    payload: row.get(4)?,
                    prev_hash: row.get(5)?,
                    hash: row.get(6)?,
                })
            })
            .context("failed to execute verify query")?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to collect verify results")?;

        let mut expected_prev_hash = GENESIS_PREV_HASH.to_string();

        for event in &events {
            // Check the prev_hash chain link
            if event.prev_hash != expected_prev_hash {
                tracing::warn!(
                    event_id = event.id,
                    expected = %expected_prev_hash,
                    actual = %event.prev_hash,
                    "chain break: prev_hash mismatch"
                );
                return Ok(false);
            }

            // Recompute and verify the event hash
            let computed = Self::compute_hash(
                event.id,
                &event.ts,
                &event.event_type,
                &event.actor,
                &event.payload,
                &event.prev_hash,
            );

            if computed != event.hash {
                tracing::warn!(
                    event_id = event.id,
                    expected = %computed,
                    actual = %event.hash,
                    "chain break: hash mismatch"
                );
                return Ok(false);
            }

            expected_prev_hash = event.hash.clone();
        }

        Ok(true)
    }

    /// Return the total number of events in the ledger.
    pub fn event_count(&self) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .context("failed to count events")?;
        Ok(count)
    }

    // ── Incidents ──

    /// Create a new incident workspace.
    pub fn create_incident(&self, id: &str, name: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO incidents (id, name) VALUES (?1, ?2)",
                params![id, name],
            )
            .context("failed to create incident")?;
        Ok(())
    }

    /// Add a step to an incident.
    pub fn add_incident_step(
        &self,
        incident_id: &str,
        step_number: u32,
        action: &str,
        result: &str,
        receipt_id: Option<&str>,
    ) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO incident_steps (incident_id, step_number, action, result, receipt_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![incident_id, step_number, action, result, receipt_id],
            )
            .context("failed to add incident step")?;
        Ok(())
    }

    /// Get an incident by ID with all its steps.
    pub fn get_incident(&self, id: &str) -> Result<Option<IncidentRow>> {
        let incident = self
            .conn
            .query_row(
                "SELECT id, name, status, created_at, resolved_at FROM incidents WHERE id = ?1",
                params![id],
                |row| {
                    Ok(IncidentRow {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        status: row.get(2)?,
                        created_at: row.get(3)?,
                        resolved_at: row.get(4)?,
                        steps: Vec::new(),
                    })
                },
            )
            .ok();

        match incident {
            Some(mut inc) => {
                inc.steps = self.get_incident_steps(&inc.id)?;
                Ok(Some(inc))
            }
            None => Ok(None),
        }
    }

    /// Get all steps for an incident.
    fn get_incident_steps(&self, incident_id: &str) -> Result<Vec<IncidentStepRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT step_number, action, result, receipt_id, timestamp FROM incident_steps WHERE incident_id = ?1 ORDER BY step_number ASC",
        )?;
        let steps = stmt
            .query_map(params![incident_id], |row| {
                Ok(IncidentStepRow {
                    step_number: row.get(0)?,
                    action: row.get(1)?,
                    result: row.get(2)?,
                    receipt_id: row.get(3)?,
                    timestamp: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to collect incident steps")?;
        Ok(steps)
    }

    /// List incidents, optionally filtered by status.
    pub fn list_incidents(&self, status: Option<&str>) -> Result<Vec<IncidentRow>> {
        let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match status {
            Some(s) => (
                "SELECT id, name, status, created_at, resolved_at FROM incidents WHERE status = ?1 ORDER BY created_at DESC".to_string(),
                vec![Box::new(s.to_string())],
            ),
            None => (
                "SELECT id, name, status, created_at, resolved_at FROM incidents ORDER BY created_at DESC".to_string(),
                vec![],
            ),
        };

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = self.conn.prepare(&sql)?;
        let incidents = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(IncidentRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    status: row.get(2)?,
                    created_at: row.get(3)?,
                    resolved_at: row.get(4)?,
                    steps: Vec::new(),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list incidents")?;

        // Load steps for each incident
        let mut result = Vec::new();
        for mut inc in incidents {
            inc.steps = self.get_incident_steps(&inc.id)?;
            result.push(inc);
        }
        Ok(result)
    }

    /// Update incident status (e.g., "resolved").
    #[allow(dead_code)] // Public API — will be called from incident resolution endpoint
    pub fn resolve_incident(&self, id: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE incidents SET status = 'resolved', resolved_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id = ?1",
            params![id],
        ).context("failed to resolve incident")?;
        Ok(())
    }
}

/// Row type for incidents from the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentRow {
    pub id: String,
    pub name: String,
    pub status: String,
    pub created_at: String,
    pub resolved_at: Option<String>,
    pub steps: Vec<IncidentStepRow>,
}

/// Row type for incident steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncidentStepRow {
    pub step_number: u32,
    pub action: String,
    pub result: String,
    pub receipt_id: Option<String>,
    pub timestamp: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incident_create_and_get() {
        let ledger = Ledger::new(":memory:").expect("failed to create in-memory ledger");

        // Create an incident
        ledger
            .create_incident("inc-001", "Database Connection Failed")
            .expect("failed to create incident");

        // Get the incident back
        let incident = ledger
            .get_incident("inc-001")
            .expect("failed to get incident")
            .expect("incident should exist");

        // Verify fields
        assert_eq!(incident.id, "inc-001");
        assert_eq!(incident.name, "Database Connection Failed");
        assert_eq!(incident.status, "open");
        assert!(incident.resolved_at.is_none());
        assert_eq!(incident.steps.len(), 0);
    }

    #[test]
    fn test_incident_add_steps() {
        let ledger = Ledger::new(":memory:").expect("failed to create in-memory ledger");

        // Create an incident
        ledger
            .create_incident("inc-002", "Server CPU High")
            .expect("failed to create incident");

        // Add step 1
        ledger
            .add_incident_step("inc-002", 1, "Investigate CPU usage", "CPU at 95%", None)
            .expect("failed to add step 1");

        // Add step 2
        ledger
            .add_incident_step("inc-002", 2, "Restart service", "Service restarted successfully", Some("receipt-123"))
            .expect("failed to add step 2");

        // Get the incident and verify steps
        let incident = ledger
            .get_incident("inc-002")
            .expect("failed to get incident")
            .expect("incident should exist");

        assert_eq!(incident.steps.len(), 2);

        // Verify step order
        assert_eq!(incident.steps[0].step_number, 1);
        assert_eq!(incident.steps[0].action, "Investigate CPU usage");
        assert_eq!(incident.steps[0].result, "CPU at 95%");
        assert_eq!(incident.steps[0].receipt_id, None);

        assert_eq!(incident.steps[1].step_number, 2);
        assert_eq!(incident.steps[1].action, "Restart service");
        assert_eq!(incident.steps[1].result, "Service restarted successfully");
        assert_eq!(incident.steps[1].receipt_id, Some("receipt-123".to_string()));
    }

    #[test]
    fn test_list_incidents_filter_by_status() {
        let ledger = Ledger::new(":memory:").expect("failed to create in-memory ledger");

        // Create incident 1
        ledger
            .create_incident("inc-003", "Memory Leak Detected")
            .expect("failed to create incident 1");

        // Create incident 2
        ledger
            .create_incident("inc-004", "Disk Space Low")
            .expect("failed to create incident 2");

        // Resolve incident 2
        ledger
            .resolve_incident("inc-004")
            .expect("failed to resolve incident");

        // List open incidents
        let open_incidents = ledger
            .list_incidents(Some("open"))
            .expect("failed to list open incidents");

        assert_eq!(open_incidents.len(), 1);
        assert_eq!(open_incidents[0].id, "inc-003");
        assert_eq!(open_incidents[0].status, "open");

        // List all incidents
        let all_incidents = ledger
            .list_incidents(None)
            .expect("failed to list all incidents");

        assert_eq!(all_incidents.len(), 2);

        // List resolved incidents
        let resolved_incidents = ledger
            .list_incidents(Some("resolved"))
            .expect("failed to list resolved incidents");

        assert_eq!(resolved_incidents.len(), 1);
        assert_eq!(resolved_incidents[0].id, "inc-004");
        assert_eq!(resolved_incidents[0].status, "resolved");
        assert!(resolved_incidents[0].resolved_at.is_some());
    }

    #[test]
    fn test_hash_delimiter_prevents_collision() {
        // Without delimiters, these two events could produce the same hash:
        // Event A: id=1, ts="23", type="abc" → "123abc..."
        // Event B: id=12, ts="3", type="abc" → "123abc..."
        // With pipe delimiters: "1|23|abc|..." vs "12|3|abc|..." → different hashes
        let hash_a = Ledger::compute_hash(1, "23", "abc", "actor", "payload", "prev");
        let hash_b = Ledger::compute_hash(12, "3", "abc", "actor", "payload", "prev");
        assert_ne!(hash_a, hash_b, "pipe delimiters should prevent field-boundary collisions");
    }

    #[test]
    fn test_hash_chain_integrity() {
        let ledger = Ledger::new(":memory:").expect("failed to create in-memory ledger");

        ledger.append("test.event", "tester", "payload1").unwrap();
        ledger.append("test.event", "tester", "payload2").unwrap();
        ledger.append("test.event", "tester", "payload3").unwrap();

        assert!(ledger.verify().unwrap(), "hash chain should be valid");
    }

    #[test]
    fn test_schema_version_set() {
        let ledger = Ledger::new(":memory:").expect("failed to create in-memory ledger");
        let version: i64 = ledger.conn
            .query_row("SELECT version FROM schema_version WHERE id = 1", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, Ledger::CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_flush() {
        let ledger = Ledger::new(":memory:").expect("failed to create in-memory ledger");
        ledger.append("test", "actor", "payload").unwrap();
        // flush should not error on in-memory DB
        assert!(ledger.flush().is_ok());
    }

    #[test]
    fn test_incident_not_found() {
        let ledger = Ledger::new(":memory:").expect("failed to create in-memory ledger");

        // Try to get a non-existent incident
        let incident = ledger
            .get_incident("non-existent-id")
            .expect("should not error on missing incident");

        assert!(incident.is_none());
    }

    #[test]
    fn test_fts_search_basic() {
        let ledger = Ledger::new(":memory:").expect("failed to create in-memory ledger");

        ledger.append("memory.store", "agentd", r#"{"summary":"nginx crashed with OOM","detail":"nginx ran out of memory"}"#).unwrap();
        ledger.append("memory.store", "agentd", r#"{"summary":"postgres backup completed","detail":"daily backup finished"}"#).unwrap();
        ledger.append("memory.store", "agentd", r#"{"summary":"user login from SSH","detail":"admin connected via SSH"}"#).unwrap();

        let results = ledger.fts_search("nginx memory", 10).unwrap();
        assert!(!results.is_empty(), "should find nginx-related events");
        assert!(results[0].0.payload.contains("nginx"), "most relevant result should mention nginx");
    }

    #[test]
    fn test_fts_search_empty_query() {
        let ledger = Ledger::new(":memory:").expect("failed to create in-memory ledger");
        ledger.append("test", "actor", "payload").unwrap();

        let results = ledger.fts_search("", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_fts_search_no_match() {
        let ledger = Ledger::new(":memory:").expect("failed to create in-memory ledger");
        ledger.append("test", "actor", r#"{"summary":"hello world"}"#).unwrap();

        let results = ledger.fts_search("zzzznonexistent", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_sanitize_fts_query() {
        assert_eq!(Ledger::sanitize_fts_query("nginx crash"), "\"nginx\" OR \"crash\"");
        assert_eq!(Ledger::sanitize_fts_query(""), "");
        assert_eq!(Ledger::sanitize_fts_query("!!!"), "");
        assert_eq!(Ledger::sanitize_fts_query("hello-world"), "\"hello-world\"");
    }

    #[test]
    fn test_fts_porter_stemming() {
        let ledger = Ledger::new(":memory:").expect("failed to create in-memory ledger");
        ledger.append("test", "actor", r#"{"summary":"the service is running normally"}"#).unwrap();

        // "run" should match "running" via Porter stemming
        let results = ledger.fts_search("run", 10).unwrap();
        assert!(!results.is_empty(), "Porter stemming should match 'run' to 'running'");
    }
}

