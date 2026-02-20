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
            );",
        )
        .context("failed to create events table")?;

        Ok(Self { conn })
    }

    /// Compute the SHA-256 hash for an event row.
    fn compute_hash(
        id: i64,
        ts: &str,
        event_type: &str,
        actor: &str,
        payload: &str,
        prev_hash: &str,
    ) -> String {
        let input = format!("{id}{ts}{event_type}{actor}{payload}{prev_hash}");
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Retrieve the hash of the most recent event, or the genesis zero-hash if empty.
    pub fn last_hash(&self) -> Result<String> {
        let maybe: Option<String> = self
            .conn
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
    /// Returns the created event including its assigned id, timestamp, and hash.
    pub fn append(&self, event_type: &str, actor: &str, payload: &str) -> Result<Event> {
        let prev_hash = self.last_hash()?;

        // Insert with server-generated timestamp; we need to read it back.
        self.conn
            .execute(
                "INSERT INTO events (type, actor, payload, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, '')",
                params![event_type, actor, payload, prev_hash],
            )
            .context("failed to insert event")?;

        let id = self.conn.last_insert_rowid();

        // Read back the server-generated timestamp.
        let ts: String = self
            .conn
            .query_row("SELECT ts FROM events WHERE id = ?1", params![id], |row| {
                row.get(0)
            })
            .context("failed to read back event timestamp")?;

        let hash = Self::compute_hash(id, &ts, event_type, actor, payload, &prev_hash);

        self.conn
            .execute(
                "UPDATE events SET hash = ?1 WHERE id = ?2",
                params![hash, id],
            )
            .context("failed to update event hash")?;

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
}
