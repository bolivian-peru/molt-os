# AgentOS Tool Reference

## agentd endpoints

All communication with the system goes through agentd, a Rust daemon running on a Unix socket at `/run/agentos/agentd.sock`.

### GET /health
Returns system health snapshot.
```json
{
  "hostname": "agentos-dev",
  "uptime_seconds": 3600,
  "cpu_usage": [12.5, 8.3, 15.2, 10.1],
  "memory_total": 4294967296,
  "memory_used": 2147483648,
  "memory_available": 2147483648,
  "swap_total": 2147483648,
  "swap_used": 0,
  "load_average": { "one": 0.5, "five": 0.3, "fifteen": 0.2 },
  "disks": [
    { "mount": "/", "total": 21474836480, "used": 5368709120, "available": 16106127360 }
  ]
}
```

### POST /system/query
Query any system state. Returns structured JSON.

**Request:**
```json
{
  "query": "processes",
  "args": { "sort": "cpu", "limit": 10 }
}
```

**Supported queries:**
- `processes` — list processes (args: sort=cpu|memory, limit, name)
- `disk` — disk usage per mount point
- `hostname` — system hostname
- `uptime` — system uptime in seconds

### GET /events/log
Query the hash-chained audit log.

**Query params:** `?type=system.query&actor=openclaw&limit=50`

### POST /memory/ingest
Ingest a new event into the memory system.

**Request:**
```json
{
  "event": {
    "category": "diagnosis",
    "subcategory": "root_cause",
    "actor": "openclaw.main",
    "summary": "High CPU caused by Docker build",
    "detail": "Docker build spawned 47 processes...",
    "metadata": { "severity": "warning", "tags": ["docker", "cpu"] }
  }
}
```

### POST /memory/recall
Search memory for relevant context.

**Request:**
```json
{
  "query": "docker issues",
  "max_results": 10,
  "timeframe": "7d"
}
```

### POST /memory/store
Explicitly store something in long-term memory.

**Request:**
```json
{
  "summary": "User prefers declarative NixOS config",
  "detail": "User has corrected agent 3 times to use configuration.nix instead of nix-env",
  "category": "user_pattern",
  "tags": ["preference", "nix"]
}
```

### GET /memory/health
Memory system status: embedding model readiness, collection size, state directory.

## OpenClaw tools (registered by agentos-bridge)

These are the 12 tools available to the AI agent through OpenClaw.
Registered via `api.registerTool()` factory pattern in `packages/agentos-bridge/index.ts`.

### agentd tools (communicate over Unix socket)

| Tool | Description |
|------|-------------|
| `system_health` | System health snapshot: CPU, RAM, disk, load average, uptime |
| `system_query` | Query system state: processes, services, network, disk, kernel params |
| `event_log` | Query the append-only hash-chained audit log |
| `memory_store` | Store important information in long-term OS memory |
| `memory_recall` | Search OS memory for past events, diagnoses, configs, errors |

### System tools (direct filesystem and process access)

| Tool | Description |
|------|-------------|
| `shell_exec` | Execute a shell command and return stdout |
| `file_read` | Read file contents from the filesystem |
| `file_write` | Write content to a file (creates parent dirs if needed) |
| `directory_list` | List directory contents with types and sizes |

### systemd tools (service and log management)

| Tool | Description |
|------|-------------|
| `service_status` | Get systemd service status, or list all services |
| `journal_logs` | Get journal logs filtered by unit, priority, time range |

### Network tools

| Tool | Description |
|------|-------------|
| `network_info` | Network interfaces (ip addr) and listening ports (ss -tlnp) |
