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

### GET /system/discover
Discover all running services on the system. Returns listening ports, systemd units, and detected service types.

Response includes: `{ found: [{ name, pid, port, protocol, detected_as, health_url, systemd_unit, memory_bytes, cpu_usage }], total_listening_ports, total_systemd_services }`

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

These are the 83 tools available to the AI agent through OpenClaw.
Registered via `api.registerTool()` factory pattern in `packages/osmoda-bridge/index.ts`.

### agentd tools (communicate over Unix socket)

| Tool | Description |
|------|-------------|
| `system_health` | System health snapshot: CPU, RAM, disk, load average, uptime |
| `system_query` | Query system state: processes, services, network, disk, kernel params |
| `system_discover` | Discover all running services: listening ports, systemd units, known service types (nginx, postgres, redis, node, etc.) |
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

### Mesh tools (via osmoda-mesh at `/run/osmoda/mesh.sock`)

P2P encrypted agent-to-agent communication. Noise_XX + X25519 + ML-KEM-768 (hybrid post-quantum).
No central server. Invite-based pairing.

| Tool | Description |
|------|-------------|
| `mesh_identity` | Get this instance's mesh identity (instance_id, public keys, capabilities) |
| `mesh_invite_create` | Create a copy-pasteable invite code for another osModa instance (default TTL: 1 hour) |
| `mesh_invite_accept` | Accept an invite code to establish encrypted P2P connection with a peer |
| `mesh_peers` | List all known mesh peers with connection state and last seen time |
| `mesh_peer_send` | Send an encrypted message to a connected peer (chat, alert, health report, command) |
| `mesh_peer_disconnect` | Disconnect and remove a mesh peer |
| `mesh_health` | Check mesh daemon health: peer count, connected count, identity status |
| `mesh_room_create` | Create a named group room for multi-peer communication |
| `mesh_room_join` | Add a connected peer to a group room |
| `mesh_room_send` | Send a message to all connected members of a group room |
| `mesh_room_history` | Retrieve recent messages from a group room |

### MCP tools (via osmoda-mcpd at `/run/osmoda/mcpd.sock`)

MCP server lifecycle management. Any MCP server declared in NixOS config becomes available to the AI.

| Tool | Description |
|------|-------------|
| `mcp_servers` | List all managed MCP servers with status, pid, restart count, and allowed domains |
| `mcp_server_start` | Start a stopped MCP server by name |
| `mcp_server_stop` | Stop a running MCP server by name |
| `mcp_server_restart` | Restart an MCP server (stop + start) |

### System learning tools (via osmoda-teachd at `/run/osmoda/teachd.sock`)

teachd runs two background loops: OBSERVE (every 30s — collects CPU, memory, service, journal data) and LEARN (every 5m — detects patterns from accumulated observations). Knowledge is stored in SQLite and persists across conversations.

Use `teach_context` at the start of troubleshooting to get relevant historical knowledge. Use `teach_patterns` to check for slow-burn issues the agent wouldn't catch in a single conversation (memory leaks over hours, recurring 3am failures, correlated events).

| Tool | Description |
|------|-------------|
| `teach_status` | teachd health: observation count, pattern count, knowledge count, loop status |
| `teach_observations` | Query raw observations (source filter: cpu/memory/service/journal, time range, limit) |
| `teach_patterns` | List detected patterns: recurring failures, resource trends, anomalies, correlations (filterable by type and min confidence) |
| `teach_knowledge` | List auto-generated knowledge docs with recommendations (filterable by category and tag) |
| `teach_knowledge_create` | Manually create a knowledge doc (title, category, content, tags) for teachd to surface later |
| `teach_context` | Get relevant knowledge for a given context string. Returns matching docs ranked by relevance within a token budget. Use this before diagnosing issues. |
| `teach_optimize_suggest` | Generate optimization suggestions from unapplied knowledge (e.g., restart a failing service) |
| `teach_optimize_apply` | Execute an approved optimization via SafeSwitch (atomic, rollback on failure) |

### App management tools (direct systemd-run)

Deploy and manage user applications as systemd transient services. Resource-limited via cgroups. Boot-persistent via JSON registry.

| Tool | Description |
|------|-------------|
| `app_deploy` | Deploy an app as a managed systemd service (DynamicUser isolation, resource limits, env vars, restart policy) |
| `app_list` | List all managed apps with live status (PID, memory, CPU, state) and configuration |
| `app_logs` | Retrieve journal logs for a managed app (supports time range, priority, line limits) |
| `app_stop` | Stop a running app (remains in registry for restart) |
| `app_restart` | Restart an app (systemctl restart if active, re-deploy from registry if inactive) |
| `app_remove` | Stop and permanently remove an app from the registry |

### Safety tools (direct shell — bypass AI)

Emergency controls that execute immediately without AI involvement. The user always has a way out.

| Tool | Description |
|------|-------------|
| `safety_rollback` | EMERGENCY: Immediate `nixos-rebuild --rollback switch` |
| `safety_status` | Raw system health dump. Tries agentd, falls back to shell if agentd is down |
| `safety_panic` | Stop all osModa services (except agentd) + rollback NixOS |
| `safety_restart` | Restart the OpenClaw gateway service |

### Wallet tools — optional (via osmoda-keyd at `/run/osmoda/keyd.sock`)

For AI agent workloads that need cryptographic signing. Not required for core system management.

| Tool | Description |
|------|-------------|
| `wallet_create` | Create a new ETH or SOL wallet (encrypted, policy-gated) |
| `wallet_list` | List all wallets with addresses, labels, and chains |
| `wallet_sign` | Sign raw bytes with a wallet (policy-gated, daily limits) |
| `wallet_send` | Build + sign an intent (returns signed data for external broadcast — not a fully-encoded transaction) |
| `wallet_build_tx` | Build + sign a real blockchain transaction (EIP-1559 for ETH, legacy transfer for SOL). Returns signed bytes ready for broadcast. Does NOT broadcast. |
| `wallet_delete` | Permanently delete a wallet (removes key file, zeroizes cached key, updates index) |
| `wallet_receipt` | Query wallet operation receipts from the audit ledger |

### Approval Gate tools (via agentd)

Enforces `approvalRequired` policy. Destructive commands (rm -rf, reboot, nix.rebuild, wallet.send, etc.) are blocked until explicitly approved. Non-destructive commands auto-approve instantly.

| Tool | Description |
|------|-------------|
| `approval_request` | Request approval for a destructive operation. Returns approval ID. Auto-approves if command is safe. |
| `approval_pending` | List all pending approval requests awaiting user decision |
| `approval_approve` | Approve a pending destructive operation by ID |
| `approval_check` | Check status of an approval request (pending, approved, denied, expired) |

### Sandbox tools (via agentd)

Ring 1/Ring 2 isolation using bubblewrap (bwrap). Ring 1 = approved apps with declared capabilities. Ring 2 = untrusted, maximum isolation (no network, minimal filesystem).

| Tool | Description |
|------|-------------|
| `sandbox_exec` | Execute a command in a sandboxed environment (specify ring level + capabilities) |
| `capability_mint` | Create a signed capability token (HMAC-SHA256) granting specific permissions to an app or tool |

### Fleet SafeSwitch tools (via osmoda-watch)

Coordinate deploys across multiple osModa instances via the mesh network. Quorum-based voting before execution. Auto-rollback on health check failure.

| Tool | Description |
|------|-------------|
| `fleet_propose` | Initiate a fleet-wide SafeSwitch deployment with specified mesh peers |
| `fleet_status` | Check fleet switch status: phase, votes, quorum progress, participant health |
| `fleet_vote` | Cast a vote (approve/deny) on a fleet switch proposal |
| `fleet_rollback` | Force rollback a fleet switch on all participating nodes |
