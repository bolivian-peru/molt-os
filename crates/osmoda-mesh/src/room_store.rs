use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// A persistent room entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredRoom {
    pub id: String,
    pub name: String,
    pub created_by: String,
    pub created_at: String,
}

/// A room member.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMember {
    pub peer_id: String,
    pub joined_at: String,
}

/// A persistent room message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredMessage {
    pub id: i64,
    pub room_id: String,
    pub sender: String,
    pub content: String,
    pub timestamp: String,
    pub msg_hash: String,
}

/// SQLite-backed room persistence layer.
pub struct RoomStore {
    conn: std::sync::Mutex<Connection>,
}

impl RoomStore {
    /// Create a new RoomStore, initializing tables if needed.
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("failed to open room store DB at {db_path}"))?;

        conn.pragma_update(None, "journal_mode", "WAL")
            .context("failed to set WAL mode")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS rooms (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                created_by TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
            );

            CREATE TABLE IF NOT EXISTS room_members (
                room_id TEXT NOT NULL REFERENCES rooms(id),
                peer_id TEXT NOT NULL,
                joined_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                PRIMARY KEY (room_id, peer_id)
            );

            CREATE TABLE IF NOT EXISTS room_messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                room_id TEXT NOT NULL REFERENCES rooms(id),
                sender TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
                msg_hash TEXT NOT NULL UNIQUE
            );

            CREATE INDEX IF NOT EXISTS idx_room_messages_room
                ON room_messages(room_id, timestamp);
            CREATE INDEX IF NOT EXISTS idx_room_messages_hash
                ON room_messages(msg_hash);",
        )
        .context("failed to create room store tables")?;

        Ok(Self {
            conn: std::sync::Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().expect("room store DB lock poisoned")
    }

    /// Create a new room.
    pub fn create_room(&self, id: &str, name: &str, created_by: &str) -> Result<StoredRoom> {
        let conn = self.conn();
        let now = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO rooms (id, name, created_by, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, name, created_by, now],
        )
        .context("failed to create room")?;

        // Creator auto-joins
        conn.execute(
            "INSERT INTO room_members (room_id, peer_id, joined_at) VALUES (?1, ?2, ?3)",
            params![id, created_by, now],
        )
        .context("failed to add creator as member")?;

        Ok(StoredRoom {
            id: id.to_string(),
            name: name.to_string(),
            created_by: created_by.to_string(),
            created_at: now,
        })
    }

    /// Join a room.
    pub fn join_room(&self, room_id: &str, peer_id: &str) -> Result<()> {
        let conn = self.conn();
        let now = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT OR IGNORE INTO room_members (room_id, peer_id, joined_at) VALUES (?1, ?2, ?3)",
            params![room_id, peer_id, now],
        )
        .context("failed to join room")?;

        Ok(())
    }

    /// Leave a room.
    pub fn leave_room(&self, room_id: &str, peer_id: &str) -> Result<()> {
        let conn = self.conn();
        conn.execute(
            "DELETE FROM room_members WHERE room_id = ?1 AND peer_id = ?2",
            params![room_id, peer_id],
        )
        .context("failed to leave room")?;
        Ok(())
    }

    /// Compute a message hash for deduplication.
    pub fn message_hash(sender: &str, content: &str, timestamp: &str) -> String {
        use sha2::{Digest, Sha256};
        let input = format!("{sender}|{content}|{timestamp}");
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Store a message, returning true if it was new (not a duplicate).
    pub fn store_message(
        &self,
        room_id: &str,
        sender: &str,
        content: &str,
        timestamp: &str,
    ) -> Result<bool> {
        let conn = self.conn();
        let hash = Self::message_hash(sender, content, timestamp);

        let result = conn.execute(
            "INSERT OR IGNORE INTO room_messages (room_id, sender, content, timestamp, msg_hash)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![room_id, sender, content, timestamp, hash],
        );

        match result {
            Ok(rows) => Ok(rows > 0),
            Err(e) => Err(e.into()),
        }
    }

    /// Get message history for a room, optionally since a timestamp.
    pub fn get_history(
        &self,
        room_id: &str,
        since: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredMessage>> {
        let conn = self.conn();

        let (sql, param_values): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match since {
            Some(ts) => (
                "SELECT id, room_id, sender, content, timestamp, msg_hash
                 FROM room_messages WHERE room_id = ?1 AND timestamp > ?2
                 ORDER BY timestamp ASC LIMIT ?3"
                    .to_string(),
                vec![
                    Box::new(room_id.to_string()),
                    Box::new(ts.to_string()),
                    Box::new(limit as i64),
                ],
            ),
            None => (
                "SELECT id, room_id, sender, content, timestamp, msg_hash
                 FROM room_messages WHERE room_id = ?1
                 ORDER BY timestamp DESC LIMIT ?2"
                    .to_string(),
                vec![
                    Box::new(room_id.to_string()),
                    Box::new(limit as i64),
                ],
            ),
        };

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let mut stmt = conn.prepare(&sql)?;
        let messages = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok(StoredMessage {
                    id: row.get(0)?,
                    room_id: row.get(1)?,
                    sender: row.get(2)?,
                    content: row.get(3)?,
                    timestamp: row.get(4)?,
                    msg_hash: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to query room messages")?;

        Ok(messages)
    }

    /// List all rooms.
    pub fn list_rooms(&self) -> Result<Vec<StoredRoom>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, name, created_by, created_at FROM rooms ORDER BY created_at DESC",
        )?;

        let rooms = stmt
            .query_map([], |row| {
                Ok(StoredRoom {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_by: row.get(2)?,
                    created_at: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to list rooms")?;

        Ok(rooms)
    }

    /// Get room members.
    pub fn get_members(&self, room_id: &str) -> Result<Vec<RoomMember>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT peer_id, joined_at FROM room_members WHERE room_id = ?1 ORDER BY joined_at",
        )?;

        let members = stmt
            .query_map(params![room_id], |row| {
                Ok(RoomMember {
                    peer_id: row.get(0)?,
                    joined_at: row.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("failed to get room members")?;

        Ok(members)
    }

    /// Check if a room exists.
    pub fn room_exists(&self, room_id: &str) -> Result<bool> {
        let conn = self.conn();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM rooms WHERE id = ?1",
            params![room_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Get the latest message timestamp for a room.
    pub fn latest_timestamp(&self, room_id: &str) -> Result<Option<String>> {
        let conn = self.conn();
        let result = conn.query_row(
            "SELECT MAX(timestamp) FROM room_messages WHERE room_id = ?1",
            params![room_id],
            |row| row.get::<_, Option<String>>(0),
        );

        match result {
            Ok(ts) => Ok(ts),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Flush WAL checkpoint.
    pub fn flush(&self) -> Result<()> {
        let conn = self.conn();
        conn.pragma_update(None, "wal_checkpoint", "TRUNCATE")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> RoomStore {
        RoomStore::new(":memory:").unwrap()
    }

    #[test]
    fn test_create_room() {
        let store = test_store();
        let room = store.create_room("room-1", "Test Room", "peer-a").unwrap();
        assert_eq!(room.id, "room-1");
        assert_eq!(room.name, "Test Room");
        assert_eq!(room.created_by, "peer-a");
    }

    #[test]
    fn test_join_room() {
        let store = test_store();
        store.create_room("room-1", "Test", "peer-a").unwrap();
        store.join_room("room-1", "peer-b").unwrap();

        let members = store.get_members("room-1").unwrap();
        assert_eq!(members.len(), 2);
    }

    #[test]
    fn test_leave_room() {
        let store = test_store();
        store.create_room("room-1", "Test", "peer-a").unwrap();
        store.join_room("room-1", "peer-b").unwrap();
        store.leave_room("room-1", "peer-b").unwrap();

        let members = store.get_members("room-1").unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].peer_id, "peer-a");
    }

    #[test]
    fn test_store_and_retrieve_messages() {
        let store = test_store();
        store.create_room("room-1", "Test", "peer-a").unwrap();

        let ts1 = "2026-01-01T00:00:00Z";
        let ts2 = "2026-01-01T00:01:00Z";

        assert!(store
            .store_message("room-1", "peer-a", "hello", ts1)
            .unwrap());
        assert!(store
            .store_message("room-1", "peer-b", "world", ts2)
            .unwrap());

        let history = store.get_history("room-1", None, 50).unwrap();
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_message_dedup() {
        let store = test_store();
        store.create_room("room-1", "Test", "peer-a").unwrap();

        let ts = "2026-01-01T00:00:00Z";
        assert!(store
            .store_message("room-1", "peer-a", "hello", ts)
            .unwrap());
        // Same message should be deduplicated
        assert!(!store
            .store_message("room-1", "peer-a", "hello", ts)
            .unwrap());

        let history = store.get_history("room-1", None, 50).unwrap();
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn test_history_since() {
        let store = test_store();
        store.create_room("room-1", "Test", "peer-a").unwrap();

        store
            .store_message("room-1", "peer-a", "old", "2026-01-01T00:00:00Z")
            .unwrap();
        store
            .store_message("room-1", "peer-b", "new", "2026-01-02T00:00:00Z")
            .unwrap();

        let history = store
            .get_history("room-1", Some("2026-01-01T12:00:00Z"), 50)
            .unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "new");
    }

    #[test]
    fn test_list_rooms() {
        let store = test_store();
        store.create_room("room-1", "Room A", "peer-a").unwrap();
        store.create_room("room-2", "Room B", "peer-b").unwrap();

        let rooms = store.list_rooms().unwrap();
        assert_eq!(rooms.len(), 2);
    }

    #[test]
    fn test_room_exists() {
        let store = test_store();
        assert!(!store.room_exists("room-1").unwrap());
        store.create_room("room-1", "Test", "peer-a").unwrap();
        assert!(store.room_exists("room-1").unwrap());
    }

    #[test]
    fn test_persist_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("rooms.db");
        let db_str = db_path.to_str().unwrap();

        {
            let store = RoomStore::new(db_str).unwrap();
            store.create_room("room-1", "Persist Test", "peer-a").unwrap();
            store
                .store_message("room-1", "peer-a", "persisted msg", "2026-01-01T00:00:00Z")
                .unwrap();
        }

        // Reopen
        let store = RoomStore::new(db_str).unwrap();
        let rooms = store.list_rooms().unwrap();
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms[0].name, "Persist Test");

        let history = store.get_history("room-1", None, 50).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "persisted msg");
    }

    #[test]
    fn test_message_hash_deterministic() {
        let h1 = RoomStore::message_hash("peer-a", "hello", "2026-01-01T00:00:00Z");
        let h2 = RoomStore::message_hash("peer-a", "hello", "2026-01-01T00:00:00Z");
        assert_eq!(h1, h2);

        let h3 = RoomStore::message_hash("peer-b", "hello", "2026-01-01T00:00:00Z");
        assert_ne!(h1, h3);
    }
}
