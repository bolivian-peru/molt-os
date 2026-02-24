use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

// ── Data Types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub source: String,
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternType {
    Recurring,
    Trend,
    Anomaly,
    Correlation,
}

impl std::fmt::Display for PatternType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Recurring => write!(f, "recurring"),
            Self::Trend => write!(f, "trend"),
            Self::Anomaly => write!(f, "anomaly"),
            Self::Correlation => write!(f, "correlation"),
        }
    }
}

impl PatternType {
    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "recurring" => Self::Recurring,
            "trend" => Self::Trend,
            "anomaly" => Self::Anomaly,
            "correlation" => Self::Correlation,
            _ => Self::Recurring,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pattern {
    pub id: String,
    pub name: String,
    pub pattern_type: PatternType,
    pub confidence: f64,
    pub observations: Vec<String>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub occurrence_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeDoc {
    pub id: String,
    pub title: String,
    pub category: String,
    pub content: String,
    pub source_patterns: Vec<String>,
    pub confidence: f64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub applied: bool,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Optimization {
    pub id: String,
    pub knowledge_doc_id: String,
    pub description: String,
    pub action: OptAction,
    pub status: OptStatus,
    pub switch_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OptAction {
    NixConfig { diff: String },
    ServiceRestart { name: String },
    Sysctl { key: String, value: String },
    Custom { command: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OptStatus {
    Suggested,
    Approved,
    Applied,
    RolledBack,
}

impl std::fmt::Display for OptStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Suggested => write!(f, "suggested"),
            Self::Approved => write!(f, "approved"),
            Self::Applied => write!(f, "applied"),
            Self::RolledBack => write!(f, "rolled_back"),
        }
    }
}

impl OptStatus {
    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "suggested" => Self::Suggested,
            "approved" => Self::Approved,
            "applied" => Self::Applied,
            "rolled_back" => Self::RolledBack,
            _ => Self::Suggested,
        }
    }
}

// ── Database Init ──

pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS observations (
            id TEXT PRIMARY KEY,
            ts TEXT NOT NULL,
            source TEXT NOT NULL,
            data TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_obs_ts ON observations(ts);
        CREATE INDEX IF NOT EXISTS idx_obs_source ON observations(source);

        CREATE TABLE IF NOT EXISTS patterns (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            pattern_type TEXT NOT NULL,
            confidence REAL NOT NULL DEFAULT 0.0,
            observations TEXT NOT NULL DEFAULT '[]',
            first_seen TEXT NOT NULL,
            last_seen TEXT NOT NULL,
            occurrence_count INTEGER NOT NULL DEFAULT 1
        );

        CREATE TABLE IF NOT EXISTS knowledge_docs (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            category TEXT NOT NULL,
            content TEXT NOT NULL,
            source_patterns TEXT NOT NULL DEFAULT '[]',
            confidence REAL NOT NULL DEFAULT 0.0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            applied INTEGER NOT NULL DEFAULT 0,
            tags TEXT NOT NULL DEFAULT '[]'
        );

        CREATE TABLE IF NOT EXISTS optimizations (
            id TEXT PRIMARY KEY,
            knowledge_doc_id TEXT NOT NULL,
            description TEXT NOT NULL,
            action TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'suggested',
            switch_id TEXT,
            created_at TEXT NOT NULL
        );
        ",
    )
    .context("failed to initialize teachd database")?;
    Ok(())
}

// ── Observation CRUD ──

pub fn insert_observation(conn: &Connection, obs: &Observation) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO observations (id, ts, source, data) VALUES (?1, ?2, ?3, ?4)",
        params![
            obs.id,
            obs.ts.to_rfc3339(),
            obs.source,
            serde_json::to_string(&obs.data)?,
        ],
    )?;
    Ok(())
}

pub fn list_observations(
    conn: &Connection,
    source: Option<&str>,
    since: Option<&str>,
    limit: u32,
) -> Result<Vec<Observation>> {
    let mut sql = "SELECT id, ts, source, data FROM observations WHERE 1=1".to_string();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(src) = source {
        sql.push_str(" AND source = ?");
        param_values.push(Box::new(src.to_string()));
    }
    if let Some(s) = since {
        sql.push_str(" AND ts >= ?");
        param_values.push(Box::new(s.to_string()));
    }
    sql.push_str(" ORDER BY ts DESC LIMIT ?");
    param_values.push(Box::new(limit));

    let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref.as_slice(), |row| {
        let ts_str: String = row.get(1)?;
        let data_str: String = row.get(3)?;
        Ok((
            row.get::<_, String>(0)?,
            ts_str,
            row.get::<_, String>(2)?,
            data_str,
        ))
    })?;

    let mut result = Vec::new();
    for row in rows {
        let (id, ts_str, source, data_str) = row?;
        let ts = DateTime::parse_from_rfc3339(&ts_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let data: serde_json::Value =
            serde_json::from_str(&data_str).unwrap_or(serde_json::Value::Null);
        result.push(Observation {
            id,
            ts,
            source,
            data,
        });
    }
    Ok(result)
}

pub fn prune_observations(conn: &Connection, older_than: &str) -> Result<usize> {
    let deleted = conn.execute(
        "DELETE FROM observations WHERE ts < ?1",
        params![older_than],
    )?;
    Ok(deleted)
}

pub fn observation_count(conn: &Connection) -> Result<u32> {
    let count: u32 = conn.query_row("SELECT COUNT(*) FROM observations", [], |row| row.get(0))?;
    Ok(count)
}

// ── Pattern CRUD ──

pub fn upsert_pattern(conn: &Connection, pattern: &Pattern) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO patterns (id, name, pattern_type, confidence, observations, first_seen, last_seen, occurrence_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            pattern.id,
            pattern.name,
            pattern.pattern_type.to_string(),
            pattern.confidence,
            serde_json::to_string(&pattern.observations)?,
            pattern.first_seen.to_rfc3339(),
            pattern.last_seen.to_rfc3339(),
            pattern.occurrence_count,
        ],
    )?;
    Ok(())
}

pub fn list_patterns(
    conn: &Connection,
    pattern_type: Option<&str>,
    min_confidence: f64,
) -> Result<Vec<Pattern>> {
    let mut sql =
        "SELECT id, name, pattern_type, confidence, observations, first_seen, last_seen, occurrence_count FROM patterns WHERE confidence >= ?"
            .to_string();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    param_values.push(Box::new(min_confidence));

    if let Some(pt) = pattern_type {
        sql.push_str(" AND pattern_type = ?");
        param_values.push(Box::new(pt.to_string()));
    }
    sql.push_str(" ORDER BY confidence DESC, last_seen DESC");

    let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref.as_slice(), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, f64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, u32>(7)?,
        ))
    })?;

    let mut result = Vec::new();
    for row in rows {
        let (id, name, pt_str, confidence, obs_str, first_str, last_str, count) = row?;
        let observations: Vec<String> =
            serde_json::from_str(&obs_str).unwrap_or_default();
        let first_seen = DateTime::parse_from_rfc3339(&first_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        let last_seen = DateTime::parse_from_rfc3339(&last_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        result.push(Pattern {
            id,
            name,
            pattern_type: PatternType::from_str_loose(&pt_str),
            confidence,
            observations,
            first_seen,
            last_seen,
            occurrence_count: count,
        });
    }
    Ok(result)
}

pub fn pattern_count(conn: &Connection) -> Result<u32> {
    let count: u32 = conn.query_row("SELECT COUNT(*) FROM patterns", [], |row| row.get(0))?;
    Ok(count)
}

// ── KnowledgeDoc CRUD ──

pub fn insert_knowledge_doc(conn: &Connection, doc: &KnowledgeDoc) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO knowledge_docs (id, title, category, content, source_patterns, confidence, created_at, updated_at, applied, tags)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            doc.id,
            doc.title,
            doc.category,
            doc.content,
            serde_json::to_string(&doc.source_patterns)?,
            doc.confidence,
            doc.created_at.to_rfc3339(),
            doc.updated_at.to_rfc3339(),
            doc.applied as i32,
            serde_json::to_string(&doc.tags)?,
        ],
    )?;
    Ok(())
}

pub fn get_knowledge_doc(conn: &Connection, id: &str) -> Result<Option<KnowledgeDoc>> {
    let mut stmt = conn.prepare(
        "SELECT id, title, category, content, source_patterns, confidence, created_at, updated_at, applied, tags
         FROM knowledge_docs WHERE id = ?1",
    )?;

    let mut rows = stmt.query_map(params![id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, f64>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, i32>(8)?,
            row.get::<_, String>(9)?,
        ))
    })?;

    match rows.next() {
        Some(Ok((id, title, category, content, sp_str, confidence, ca_str, ua_str, applied, tags_str))) => {
            Ok(Some(parse_knowledge_doc_row(
                id, title, category, content, sp_str, confidence, ca_str, ua_str, applied, tags_str,
            )))
        }
        _ => Ok(None),
    }
}

pub fn list_knowledge_docs(
    conn: &Connection,
    category: Option<&str>,
    tag: Option<&str>,
    limit: u32,
) -> Result<Vec<KnowledgeDoc>> {
    let mut sql = "SELECT id, title, category, content, source_patterns, confidence, created_at, updated_at, applied, tags FROM knowledge_docs WHERE 1=1".to_string();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(cat) = category {
        sql.push_str(" AND category = ?");
        param_values.push(Box::new(cat.to_string()));
    }
    if let Some(t) = tag {
        sql.push_str(" AND tags LIKE ?");
        param_values.push(Box::new(format!("%\"{}\"%", t)));
    }
    sql.push_str(" ORDER BY updated_at DESC LIMIT ?");
    param_values.push(Box::new(limit));

    let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref.as_slice(), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, f64>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, i32>(8)?,
            row.get::<_, String>(9)?,
        ))
    })?;

    let mut result = Vec::new();
    for row in rows {
        let (id, title, category, content, sp_str, confidence, ca_str, ua_str, applied, tags_str) = row?;
        result.push(parse_knowledge_doc_row(
            id, title, category, content, sp_str, confidence, ca_str, ua_str, applied, tags_str,
        ));
    }
    Ok(result)
}

pub fn knowledge_count(conn: &Connection) -> Result<u32> {
    let count: u32 =
        conn.query_row("SELECT COUNT(*) FROM knowledge_docs", [], |row| row.get(0))?;
    Ok(count)
}

fn parse_knowledge_doc_row(
    id: String,
    title: String,
    category: String,
    content: String,
    sp_str: String,
    confidence: f64,
    ca_str: String,
    ua_str: String,
    applied: i32,
    tags_str: String,
) -> KnowledgeDoc {
    let source_patterns: Vec<String> = serde_json::from_str(&sp_str).unwrap_or_default();
    let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
    let created_at = DateTime::parse_from_rfc3339(&ca_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    let updated_at = DateTime::parse_from_rfc3339(&ua_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    KnowledgeDoc {
        id,
        title,
        category,
        content,
        source_patterns,
        confidence,
        created_at,
        updated_at,
        applied: applied != 0,
        tags,
    }
}

// ── Optimization CRUD ──

pub fn insert_optimization(conn: &Connection, opt: &Optimization) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO optimizations (id, knowledge_doc_id, description, action, status, switch_id, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            opt.id,
            opt.knowledge_doc_id,
            opt.description,
            serde_json::to_string(&opt.action)?,
            opt.status.to_string(),
            opt.switch_id,
            opt.created_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub fn get_optimization(conn: &Connection, id: &str) -> Result<Option<Optimization>> {
    let mut stmt = conn.prepare(
        "SELECT id, knowledge_doc_id, description, action, status, switch_id, created_at
         FROM optimizations WHERE id = ?1",
    )?;

    let mut rows = stmt.query_map(params![id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;

    match rows.next() {
        Some(Ok((id, kid, desc, action_str, status_str, switch_id, ca_str))) => {
            let action: OptAction = serde_json::from_str(&action_str)
                .unwrap_or(OptAction::Custom { command: action_str });
            let created_at = DateTime::parse_from_rfc3339(&ca_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            Ok(Some(Optimization {
                id,
                knowledge_doc_id: kid,
                description: desc,
                action,
                status: OptStatus::from_str_loose(&status_str),
                switch_id,
                created_at,
            }))
        }
        _ => Ok(None),
    }
}

pub fn list_optimizations(
    conn: &Connection,
    status: Option<&str>,
    limit: u32,
) -> Result<Vec<Optimization>> {
    let mut sql = "SELECT id, knowledge_doc_id, description, action, status, switch_id, created_at FROM optimizations WHERE 1=1".to_string();
    let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(s) = status {
        sql.push_str(" AND status = ?");
        param_values.push(Box::new(s.to_string()));
    }
    sql.push_str(" ORDER BY created_at DESC LIMIT ?");
    param_values.push(Box::new(limit));

    let params_ref: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_ref.as_slice(), |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;

    let mut result = Vec::new();
    for row in rows {
        let (id, kid, desc, action_str, status_str, switch_id, ca_str) = row?;
        let action: OptAction = serde_json::from_str(&action_str)
            .unwrap_or(OptAction::Custom { command: action_str });
        let created_at = DateTime::parse_from_rfc3339(&ca_str)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
        result.push(Optimization {
            id,
            knowledge_doc_id: kid,
            description: desc,
            action,
            status: OptStatus::from_str_loose(&status_str),
            switch_id,
            created_at,
        });
    }
    Ok(result)
}

pub fn update_optimization_status(
    conn: &Connection,
    id: &str,
    status: &OptStatus,
    switch_id: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE optimizations SET status = ?1, switch_id = ?2 WHERE id = ?3",
        params![status.to_string(), switch_id, id],
    )?;
    Ok(())
}

pub fn optimization_count(conn: &Connection) -> Result<u32> {
    let count: u32 =
        conn.query_row("SELECT COUNT(*) FROM optimizations", [], |row| row.get(0))?;
    Ok(count)
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        conn
    }

    #[test]
    fn test_observation_crud() {
        let conn = test_db();
        let obs = Observation {
            id: "obs-1".to_string(),
            ts: Utc::now(),
            source: "cpu".to_string(),
            data: serde_json::json!({"usage": 45.2}),
        };
        insert_observation(&conn, &obs).unwrap();
        let list = list_observations(&conn, Some("cpu"), None, 10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].source, "cpu");
    }

    #[test]
    fn test_pattern_crud() {
        let conn = test_db();
        let now = Utc::now();
        let pattern = Pattern {
            id: "pat-1".to_string(),
            name: "high-cpu".to_string(),
            pattern_type: PatternType::Recurring,
            confidence: 0.85,
            observations: vec!["obs-1".to_string()],
            first_seen: now,
            last_seen: now,
            occurrence_count: 3,
        };
        upsert_pattern(&conn, &pattern).unwrap();
        let list = list_patterns(&conn, None, 0.5).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "high-cpu");
    }

    #[test]
    fn test_knowledge_doc_crud() {
        let conn = test_db();
        let now = Utc::now();
        let doc = KnowledgeDoc {
            id: "kd-1".to_string(),
            title: "CPU spike pattern".to_string(),
            category: "performance".to_string(),
            content: "System experiences CPU spikes every 5 minutes.".to_string(),
            source_patterns: vec!["pat-1".to_string()],
            confidence: 0.9,
            created_at: now,
            updated_at: now,
            applied: false,
            tags: vec!["cpu".to_string(), "performance".to_string()],
        };
        insert_knowledge_doc(&conn, &doc).unwrap();

        let fetched = get_knowledge_doc(&conn, "kd-1").unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().title, "CPU spike pattern");

        let list = list_knowledge_docs(&conn, Some("performance"), None, 10).unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn test_optimization_crud() {
        let conn = test_db();
        let opt = Optimization {
            id: "opt-1".to_string(),
            knowledge_doc_id: "kd-1".to_string(),
            description: "Restart nginx".to_string(),
            action: OptAction::ServiceRestart {
                name: "nginx".to_string(),
            },
            status: OptStatus::Suggested,
            switch_id: None,
            created_at: Utc::now(),
        };
        insert_optimization(&conn, &opt).unwrap();

        update_optimization_status(&conn, "opt-1", &OptStatus::Approved, None).unwrap();
        let fetched = get_optimization(&conn, "opt-1").unwrap().unwrap();
        assert_eq!(fetched.status, OptStatus::Approved);
    }

    #[test]
    fn test_prune_observations() {
        let conn = test_db();
        let old_obs = Observation {
            id: "old-1".to_string(),
            ts: DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            source: "cpu".to_string(),
            data: serde_json::json!({}),
        };
        let new_obs = Observation {
            id: "new-1".to_string(),
            ts: Utc::now(),
            source: "cpu".to_string(),
            data: serde_json::json!({}),
        };
        insert_observation(&conn, &old_obs).unwrap();
        insert_observation(&conn, &new_obs).unwrap();

        let pruned = prune_observations(&conn, "2025-01-01T00:00:00Z").unwrap();
        assert_eq!(pruned, 1);

        let remaining = list_observations(&conn, None, None, 10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "new-1");
    }
}
