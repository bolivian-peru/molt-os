use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "agentctl", about = "osModa CLI — query events and verify ledger integrity")]
struct Cli {
    /// Path to the agentd state directory
    #[arg(long, default_value = "/var/lib/osmoda")]
    state_dir: PathBuf,

    /// Path to the agentd Unix socket
    #[arg(long, default_value = "/run/osmoda/agentd.sock")]
    socket: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show recent events from the ledger
    Events {
        /// Number of events to show
        #[arg(long, default_value = "20")]
        last: u32,

        /// Filter by event type
        #[arg(long)]
        r#type: Option<String>,

        /// Filter by actor
        #[arg(long)]
        actor: Option<String>,
    },

    /// Verify the integrity of the hash-chained ledger
    VerifyLedger,

    /// Show ledger statistics
    Stats,

    /// Query agentd health endpoint
    Health,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Events { last, r#type, actor } => {
            cmd_events(&cli.state_dir, last, r#type, actor)
        }
        Commands::VerifyLedger => cmd_verify_ledger(&cli.state_dir),
        Commands::Stats => cmd_stats(&cli.state_dir),
        Commands::Health => cmd_health(&cli.socket),
    }
}

fn open_ledger(state_dir: &PathBuf) -> Result<Connection> {
    let db_path = state_dir.join("ledger.db");
    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open ledger at {}", db_path.display()))?;
    Ok(conn)
}

fn cmd_events(
    state_dir: &PathBuf,
    last: u32,
    type_filter: Option<String>,
    actor_filter: Option<String>,
) -> Result<()> {
    let conn = open_ledger(state_dir)?;

    let mut query = String::from(
        "SELECT id, ts, type, actor, payload, hash FROM events WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    if let Some(ref t) = type_filter {
        query.push_str(" AND type = ?");
        params.push(Box::new(t.clone()));
    }
    if let Some(ref a) = actor_filter {
        query.push_str(" AND actor = ?");
        params.push(Box::new(a.clone()));
    }

    query.push_str(" ORDER BY id DESC LIMIT ?");
    params.push(Box::new(last));

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&query)?;
    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
        ))
    })?;

    let mut events: Vec<_> = rows.collect::<Result<Vec<_>, _>>()?;
    events.reverse();

    for (id, ts, event_type, actor, payload, hash) in &events {
        println!("#{id} [{ts}] {event_type} by {actor}");
        // Pretty-print payload if it's valid JSON
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(payload) {
            println!(
                "  {}",
                serde_json::to_string_pretty(&val)
                    .unwrap_or_else(|_| payload.clone())
                    .replace('\n', "\n  ")
            );
        } else {
            println!("  {payload}");
        }
        println!("  hash: {hash}");
        println!();
    }

    println!("Showed {} event(s)", events.len());
    Ok(())
}

fn cmd_verify_ledger(state_dir: &PathBuf) -> Result<()> {
    let conn = open_ledger(state_dir)?;

    let mut stmt =
        conn.prepare("SELECT id, ts, type, actor, payload, prev_hash, hash FROM events ORDER BY id ASC")?;

    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
        ))
    })?;

    let mut expected_prev_hash = "0".repeat(64); // matches GENESIS_PREV_HASH in agentd ledger
    let mut count = 0u64;
    let mut errors = 0u64;

    for row in rows {
        let (id, ts, event_type, actor, payload, prev_hash, stored_hash) = row?;
        count += 1;

        // Verify prev_hash links correctly
        if prev_hash != expected_prev_hash {
            eprintln!(
                "CHAIN BREAK at event #{id}: expected prev_hash={expected_prev_hash}, got {prev_hash}"
            );
            errors += 1;
        }

        // Recompute hash (pipe-delimited to match agentd ledger format)
        let hash_input = format!("{id}|{ts}|{event_type}|{actor}|{payload}|{prev_hash}");
        let mut hasher = Sha256::new();
        hasher.update(hash_input.as_bytes());
        let computed_hash = hex::encode(hasher.finalize());

        if computed_hash != stored_hash {
            eprintln!(
                "HASH MISMATCH at event #{id}: computed={computed_hash}, stored={stored_hash}"
            );
            errors += 1;
        }

        expected_prev_hash = stored_hash;
    }

    if errors == 0 {
        println!("Ledger verified: {count} events, all hashes valid, chain intact.");
    } else {
        eprintln!("LEDGER CORRUPTION: {errors} error(s) found in {count} events!");
        std::process::exit(1);
    }

    Ok(())
}

fn cmd_stats(state_dir: &PathBuf) -> Result<()> {
    let conn = open_ledger(state_dir)?;

    let total: i64 = conn.query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))?;

    let first_ts: Option<String> = conn
        .query_row("SELECT ts FROM events ORDER BY id ASC LIMIT 1", [], |row| {
            row.get(0)
        })
        .ok();

    let last_ts: Option<String> = conn
        .query_row(
            "SELECT ts FROM events ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok();

    println!("Ledger statistics:");
    println!("  Total events: {total}");
    if let Some(ts) = first_ts {
        println!("  First event:  {ts}");
    }
    if let Some(ts) = last_ts {
        println!("  Last event:   {ts}");
    }

    // Event type breakdown
    let mut stmt = conn.prepare(
        "SELECT type, COUNT(*) as cnt FROM events GROUP BY type ORDER BY cnt DESC",
    )?;
    let type_rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;

    println!("  By type:");
    for row in type_rows {
        let (event_type, cnt) = row?;
        println!("    {event_type}: {cnt}");
    }

    // DB file size
    let db_path = state_dir.join("ledger.db");
    if let Ok(meta) = std::fs::metadata(&db_path) {
        let size_mb = meta.len() as f64 / 1_048_576.0;
        println!("  DB size: {size_mb:.2} MB");
    }

    Ok(())
}

fn cmd_health(socket: &PathBuf) -> Result<()> {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    if !socket.exists() {
        anyhow::bail!(
            "agentd socket not found at {}. Is agentd running?",
            socket.display()
        );
    }

    let mut stream = UnixStream::connect(socket)
        .with_context(|| format!("Failed to connect to agentd at {}", socket.display()))?;

    // HTTP/1.0 so server closes connection after response (no keep-alive needed)
    stream.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .context("Failed to send health request")?;

    let mut response = String::new();
    stream.read_to_string(&mut response)
        .context("Failed to read health response")?;

    // Strip HTTP headers — body starts after first blank line
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or(&response);

    let val: serde_json::Value = serde_json::from_str(body)
        .with_context(|| format!("Failed to parse health response: {body}"))?;

    println!("{}", serde_json::to_string_pretty(&val)?);
    Ok(())
}
