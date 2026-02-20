---
name: flight-recorder
description: >
  Black box flight recorder for the server. Capture system state snapshots
  for forensic analysis after incidents. Continuous telemetry ring buffer.
tools:
  - system_health
  - shell_exec
  - journal_logs
  - event_log
  - memory_store
  - memory_recall
activation: auto
---

# Flight Recorder — Server Black Box

Continuous state capture for post-incident forensics.

## Snapshot Capture (run every 5 minutes via heartbeat)

Capture a lightweight system snapshot:
```
system_health()
shell_exec({ command: "ps aux --sort=-%mem | head -10" })
shell_exec({ command: "ss -s" })  # connection summary
```

Store as a compact memory entry:
```
memory_store({
  summary: "Flight recorder: CPU 12%, RAM 4.1G, load 0.3, 142 connections",
  detail: "Top processes: postgres 1.2G, openclaw 380M, node 210M. Net: 142 established, 12 listening.",
  category: "system.config",
  tags: "flight-recorder,snapshot,telemetry"
})
```

## Post-Incident Analysis

When investigating "what happened at 3 AM":

### 1. Pull flight recorder snapshots
```
memory_recall({ query: "flight-recorder snapshot", timeframe: "24h", max_results: 50 })
```

### 2. Pull journal logs for the incident window
```
journal_logs({ since: "2 hours ago", priority: "warning", lines: 100 })
```

### 3. Pull audit ledger
```
event_log({ limit: 50 })
```

### 4. Reconstruct the incident

Present a forensic timeline:
```
Incident Report — Server Crash at 03:12 UTC

Timeline:
  02:00  Flight recorder: CPU 15%, RAM 5.2G (normal)
  02:30  Flight recorder: CPU 23%, RAM 5.8G (rising)
  02:47  Flight recorder: CPU 89%, RAM 7.1G (critical)
  02:47  OOM killer triggered (killed: postgres worker)
  02:48  PostgreSQL: 3 connections terminated
  02:49  App health check failed → Caddy returned 502
  02:51  systemd restarted PostgreSQL automatically
  02:52  All services recovered
  02:55  Flight recorder: CPU 12%, RAM 4.1G (normal)

Root cause: PostgreSQL autovacuum on users table (12M rows)
during traffic spike. Memory peaked at 7.4/7.6 GB.

Recommendations:
  1. Add 2GB swap (NixOS config change)
  2. Schedule autovacuum for low-traffic hours
  3. Consider 16GB RAM plan ($6/mo more on Hetzner)

All data sourced from flight recorder snapshots,
journal logs, and audit ledger. Chain verified.
```

## Data Retention

Flight recorder snapshots are compact (~200 bytes each).
At 5-minute intervals: 288/day, ~56KB/day, ~1.7MB/month.
Keep 30 days of snapshots. Memory recall handles the rest.
