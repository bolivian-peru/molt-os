# osModa Tool Reference

## agentd endpoints

All communication with the system goes through agentd, a Rust daemon running on a Unix socket at `/run/osmoda/agentd.sock`.

### GET /health
Returns system health snapshot.
```json
{
  "hostname": "osmoda-dev",
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

## OpenClaw tools (registered by osmoda-bridge)

These are the 37 tools available to the AI agent through OpenClaw.
Registered via `api.registerTool()` factory pattern in `packages/osmoda-bridge/index.ts`.

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
| `shell_exec` | Execute a shell command and return stdout. Timeout capped at 120s. Dangerous commands are logged to the audit ledger. |
| `file_read` | Read file contents from the filesystem. Restricted to /var/lib/osmoda/, /etc/nixos/, /home/, /tmp/, /etc/, /var/log/. Rejects path traversal. |
| `file_write` | Write content to a file (creates parent dirs if needed). Same path restrictions as file_read. Uses atomic writes (write to .tmp then rename). |
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

### Wallet tools (via osmoda-keyd at `/run/osmoda/keyd.sock`)

| Tool | Description |
|------|-------------|
| `wallet_create` | Create a new ETH or SOL wallet (encrypted, policy-gated) |
| `wallet_list` | List all wallets with addresses, labels, and chains |
| `wallet_sign` | Sign raw bytes with a wallet (policy-gated, daily limits) |
| `wallet_send` | Build + sign a transaction (returns signed tx for external broadcast) |
| `wallet_delete` | Permanently delete a wallet (removes key file, zeroizes cached key, updates index) |
| `wallet_receipt` | Query wallet operation receipts from the audit ledger |

### SafeSwitch tools (via osmoda-watch at `/run/osmoda/watch.sock`)

| Tool | Description |
|------|-------------|
| `safe_switch_begin` | Start a deploy transaction with health checks + TTL + auto-rollback |
| `safe_switch_status` | Check probation status of a switch session |
| `safe_switch_commit` | Manually commit a switch session |
| `safe_switch_rollback` | Manually rollback to the previous NixOS generation |

### Watcher tools (via osmoda-watch)

| Tool | Description |
|------|-------------|
| `watcher_add` | Add an autopilot watcher with escalation actions (restart → rollback → notify) |
| `watcher_list` | List active watchers and their current health state |

### Routines tools (via osmoda-routines at `/run/osmoda/routines.sock`)

| Tool | Description |
|------|-------------|
| `routine_add` | Schedule a recurring background task (cron, interval, or event-based) |
| `routine_list` | List all scheduled routines with run history |
| `routine_trigger` | Manually trigger a routine to run immediately |

### Identity tools (via agentd)

| Tool | Description |
|------|-------------|
| `agent_card` | Get or generate the EIP-8004 Agent Card (identity + capabilities) |

### Receipt + Incident tools (via agentd)

| Tool | Description |
|------|-------------|
| `receipt_list` | Query structured receipts from the audit ledger |
| `incident_create` | Create an incident workspace for structured troubleshooting |
| `incident_step` | Add a step to an incident workspace (resumable — Shannon pattern) |

### Backup tools (via agentd)

| Tool | Description |
|------|-------------|
| `backup_create` | Create timestamped backup of all osModa state (SQLite WAL checkpoint + copy) |
| `backup_list` | List available backups with IDs, sizes, and timestamps |

### Channel management (via shell_exec + file_write)

Set up messaging channels so the user can talk to you from Telegram or WhatsApp.

| Action | How |
|--------|-----|
| Save Telegram token | `file_write` to `/var/lib/osmoda/secrets/telegram-bot-token` |
| Enable Telegram | `shell_exec`: `openclaw config set channels.telegram.enabled true` |
| Set Telegram token path | `shell_exec`: `openclaw config set channels.telegram.tokenFile /var/lib/osmoda/secrets/telegram-bot-token` |
| Restrict Telegram users | `shell_exec`: `openclaw config set channels.telegram.allowedUsers '["username"]'` |
| Enable WhatsApp | `shell_exec`: `openclaw config set channels.whatsapp.enabled true` |
| Set WhatsApp cred dir | `shell_exec`: `openclaw config set channels.whatsapp.credentialDir /var/lib/osmoda/whatsapp` |
| Restrict WhatsApp numbers | `shell_exec`: `openclaw config set channels.whatsapp.allowedNumbers '["+1234567890"]'` |
| Apply channel changes | `shell_exec`: `systemctl restart osmoda-gateway` |
| Check for WhatsApp QR | `shell_exec`: `journalctl -u osmoda-gateway --since '30 sec ago' --no-pager` |

### Voice tools (via osmoda-voice at `/run/osmoda/voice.sock`)

100% local, open-source. STT via whisper.cpp, TTS via piper-tts, audio via PipeWire.
No cloud APIs. No data leaves the machine.

| Tool | Description |
|------|-------------|
| `voice_status` | Check voice daemon status: listening state, model availability |
| `voice_speak` | Speak text aloud via piper-tts (local TTS, plays through PipeWire) |
| `voice_transcribe` | Transcribe a WAV audio file to text via whisper.cpp (local STT) |
| `voice_record` | Record audio from microphone via PipeWire, optionally transcribe |
| `voice_listen` | Enable/disable continuous listening mode |
