# Skill Auto-Learning — How osModa Teaches Itself

osModa's teachd daemon doesn't just detect patterns — it learns from the agent's own behavior and generates executable skills automatically.

## How It Works

The agent uses tools to manage your system. Every tool execution is logged. When teachd detects the agent performing the same tool sequence across multiple sessions, it creates a **skill candidate** — a reusable procedure the agent can follow next time instead of reasoning from scratch.

```
Session 1: journal_logs → service_status → shell_exec (restart nginx after OOM)
Session 2: journal_logs → service_status → shell_exec (restart postgres after OOM)
Session 3: journal_logs → service_status → shell_exec (restart redis after crash)

teachd detects: "This 3-tool sequence appeared in 3+ sessions"
     → Creates skill candidate: "check-logs-then-restart"
     → Agent reviews and generates SKILL.md
     → Skill available for immediate use next time
```

## The Pipeline

```
1. OBSERVE  — Agent uses tools → logged to agent_actions table
2. DETECT   — SKILLGEN loop (every 6h) finds repeated tool sequences
3. REVIEW   — Skill candidates listed for agent/user review
4. GENERATE — SKILL.md file written with step-by-step procedure
5. PROMOTE  — After validation, activation set to auto
6. TRACK    — Success/failure recorded, confidence updated
```

### Step 1: Action Logging

Every tool the agent executes is logged via `POST /observe/action`:

```json
{
  "tool": "journal_logs",
  "params": { "unit": "nginx", "lines": 20 },
  "result_summary": "Found OOM kill entries",
  "session_id": "sess-abc123",
  "success": true
}
```

Actions are stored in SQLite with a 30-day retention window.

### Step 2: Sequence Detection

The SKILLGEN background loop runs every 6 hours. It:

1. Groups actions by session, ordered by timestamp
2. Extracts contiguous subsequences of 3-6 tools
3. Counts how many distinct sessions contain each sequence
4. Filters to sequences appearing in **3+ sessions** (configurable)
5. Deduplicates — removes subsequences dominated by longer patterns
6. Scores by frequency and sequence length

### Step 3: Candidate Review

Skill candidates appear in the `teach_skill_candidates` tool:

```json
{
  "id": "sc-journal_logs-service_status-shell_exec",
  "name": "journal_logs-and-status-and-exec",
  "description": "Automated procedure using journal_logs, then service_status, then shell_exec in sequence.",
  "tools": ["journal_logs", "service_status", "shell_exec"],
  "session_count": 5,
  "confidence": 0.74,
  "status": "pending"
}
```

### Step 4: SKILL.md Generation

When a candidate is approved via `teach_skill_generate`, teachd writes a SKILL.md file:

```yaml
---
name: journal_logs-and-status-and-exec
description: >
  Automated procedure using journal_logs, then service_status, then shell_exec in sequence.
  Auto-generated from 5 observed sessions on 2026-03-08.
tools:
  - journal_logs
  - service_status
  - shell_exec
activation: manual
auto_generated: true
version: 1
confidence: 0.74
---
```

Skills start with `activation: manual` — they must be explicitly invoked until promoted.

### Step 5: Promotion

After validating that a skill works correctly, `teach_skill_promote` sets `activation: auto`. The SKILL.md file is updated in place.

### Step 6: Success Tracking

Every skill execution is recorded via `teach_skill_execution`:

```json
{
  "skill_name": "journal_logs-and-status-and-exec",
  "outcome": "success",
  "session_id": "sess-xyz789",
  "notes": "Resolved nginx OOM, service back up"
}
```

Success rate is tracked per skill. Skills with consistently low success rates (<30% after 5+ executions) should be reviewed or retired.

## Quality Gates

Before creating a skill candidate, the generator validates:

| Gate | Threshold |
|------|-----------|
| Minimum sessions | 3+ distinct sessions with the same sequence |
| Tool overlap | <80% overlap with existing candidates (prevents duplicates) |
| Sequence length | 3-6 tools (shorter = noise, longer = too specific) |

## Confidence Scoring

Confidence is computed from two factors:

- **Session frequency** (70% weight): More sessions = higher confidence. Capped at 8 sessions = 1.0.
- **Sequence length** (30% weight): Shorter sequences are more reliable patterns.

## API Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/observe/action` | POST | Log agent tool execution |
| `/actions` | GET | List logged actions (?tool, ?session_id, ?since, ?limit) |
| `/skills/candidates` | GET | List skill candidates (?status, ?limit) |
| `/skills/generate/{id}` | POST | Generate SKILL.md from candidate |
| `/skills/promote/{id}` | POST | Set activation to auto |
| `/skills/execution` | POST | Record skill execution outcome |
| `/skills/executions` | GET | List execution history (?skill_name, ?limit) |

## Bridge Tools

| Tool | Description |
|------|-------------|
| `teach_observe_action` | Log a tool execution for skill learning |
| `teach_skill_candidates` | List detected skill candidates |
| `teach_skill_generate` | Generate SKILL.md from a candidate |
| `teach_skill_promote` | Promote skill to auto-activation |
| `teach_skill_execution` | Record execution outcome for tracking |

## Database Schema

Three new tables in teachd's SQLite database:

```sql
-- Tool executions logged by the agent
CREATE TABLE agent_actions (
    id TEXT PRIMARY KEY,
    ts TEXT NOT NULL,
    session_id TEXT NOT NULL,
    tool TEXT NOT NULL,
    params TEXT NOT NULL DEFAULT '{}',
    result_summary TEXT,
    context TEXT,
    success INTEGER NOT NULL DEFAULT 1
);

-- Detected skill candidates
CREATE TABLE skill_candidates (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    tools TEXT NOT NULL DEFAULT '[]',
    session_count INTEGER NOT NULL DEFAULT 0,
    confidence REAL NOT NULL DEFAULT 0.0,
    source_patterns TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL DEFAULT 'pending',
    skill_path TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Execution outcome tracking
CREATE TABLE skill_executions (
    id TEXT PRIMARY KEY,
    skill_name TEXT NOT NULL,
    session_id TEXT NOT NULL,
    ts TEXT NOT NULL,
    outcome TEXT NOT NULL DEFAULT 'success',
    notes TEXT
);
```

## Data Retention

- **Agent actions**: 30-day retention, auto-pruned by the observer loop
- **Skill candidates**: Permanent (small volume)
- **Skill executions**: Permanent (small volume)
- **Generated SKILL.md files**: Written to `/var/lib/osmoda/skills/auto/<name>/SKILL.md`

## Integration Points

- **osmoda-bridge**: Logs tool executions to teachd after every tool call
- **agentd ledger**: All skill events (candidate detection, generation, promotion, execution) logged as audit events
- **SKILL.md format**: Same YAML frontmatter + markdown format as hand-written skills — fully compatible with existing skill infrastructure
