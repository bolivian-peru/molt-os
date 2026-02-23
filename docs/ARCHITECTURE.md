# osModa Architecture

## Overview

osModa is a NixOS distribution where the AI agent IS the operating system interface. The agent has root access through `agentd`, a Rust daemon that provides structured, audited access to every aspect of the Linux system. Additional daemons handle crypto wallets (keyd), deploy transactions (watch), and background automation (routines).

All inter-daemon communication happens over Unix sockets. No TCP between components.

## Trust Rings

```
RING 0: OpenClaw + agentd + keyd + watch + routines
  Full system access. Root-equivalent. See and control everything.
  Components: OpenClaw Gateway, osmoda-bridge, agentd, keyd, watch, routines

RING 1: Approved Apps
  Sandboxed with declared capabilities. No root, no arbitrary filesystem.
  Execution: bubblewrap + systemd transient units
  Network: egress proxy with domain allowlist

RING 2: Untrusted Execution
  Maximum isolation. Working directory + /tmp only. No network.
  User scripts, pip packages, npm installs, third-party binaries.
```

## Component Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ User (Terminal / Browser / Chat)                             │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│ OpenClaw Gateway (:18789)                                    │
│   AI reasoning → builds prompt → calls Claude API            │
│   osmoda-bridge plugin → 37 tools registered                 │
│   Memory Backend → ZVEC search → injects into prompt (M1+)  │
└──────┬──────────┬───────────┬──────────┬────────────────────┘
       │          │           │          │
       ▼          ▼           ▼          ▼
┌──────────┐ ┌──────────┐ ┌─────────┐ ┌───────────┐
│ agentd   │ │ keyd     │ │ watch   │ │ routines  │
│          │ │          │ │         │ │           │
│ /health  │ │ /wallet/ │ │/switch/ │ │/routine/  │
│ /system/ │ │  create  │ │ begin   │ │ add       │
│ /events/ │ │  list    │ │ status  │ │ list      │
│ /memory/ │ │  sign    │ │ commit  │ │ trigger   │
│ /agent/  │ │  send    │ │rollback │ │ history   │
│ /receipt │ │          │ │/watcher/│ │           │
│ /incidnt │ │ Private  │ │ add     │ │ Scheduler │
│          │ │ Network  │ │ list    │ │ loop runs │
│ Ledger   │ │ (no net) │ │ remove  │ │ every 60s │
│ (SQLite) │ │          │ │         │ │           │
│          │ │ AES-256  │ │ Health  │ │ Cron +    │
│ Hash     │ │ GCM keys │ │ checks  │ │ interval  │
│ chain    │ │          │ │ + auto  │ │ triggers  │
│          │ │ Policy   │ │rollback │ │           │
│          │ │ engine   │ │         │ │           │
└──────────┘ └──────────┘ └─────────┘ └───────────┘
  agentd.sock  keyd.sock  watch.sock  routines.sock
  (root)       (root,     (root)      (root)
               no network)

All sockets under /run/osmoda/
```

## Daemon Details

### agentd — Kernel Bridge

- **Socket**: `/run/osmoda/agentd.sock`
- **State**: `/var/lib/osmoda/`
- **Role**: Central daemon. Provides system queries, audit ledger, memory endpoints, Agent Card (EIP-8004), receipts, and incident workspaces.
- **Ledger**: Append-only SQLite with SHA-256 hash chaining (pipe-delimited format). Every event references the previous hash. Chain verifiable with `agentctl verify-ledger`.
- **Backup**: Daily systemd timer backs up SQLite state with WAL checkpointing. 7-day retention with automatic cleanup.
- **Hardening**: Graceful shutdown (SIGTERM/SIGINT), subprocess timeout protection, input validation with path traversal rejection.

### osmoda-keyd — Crypto Wallets

- **Socket**: `/run/osmoda/keyd.sock` (permissions: 0600)
- **State**: `/var/lib/osmoda/keyd/`
- **Role**: Generates and stores ETH (secp256k1/Keccak-256) and SOL (ed25519) wallets. Private keys encrypted with AES-256-GCM under a master key.
- **Isolation**: `PrivateNetwork=true`, `RestrictAddressFamilies=AF_UNIX`. Zero network access by design. Signed transactions must be broadcast by the caller.
- **Policy**: JSON rules file with daily spend limits, signing caps, destination allowlists. First matching rule wins.
- **Hardening**: Graceful shutdown with key zeroization, subprocess timeouts.

### osmoda-watch — SafeSwitch + Watchers

- **Socket**: `/run/osmoda/watch.sock` (permissions: 0660)
- **Role**: Deploy transaction manager and autopilot health watcher.
- **SafeSwitch flow**:
  1. Caller applies NixOS change
  2. `POST /switch/begin` records session with TTL + health checks
  3. Background loop checks health every 5 seconds
  4. If all checks pass for TTL duration → auto-commit
  5. If any check fails → auto-rollback to previous NixOS generation
  6. Receipt logged to agentd ledger
- **Watchers**: Deterministic health checks (systemd units, TCP ports, HTTP endpoints, custom commands) with escalation ladder: restart → rollback → notify.
- **Hardening**: Graceful shutdown, subprocess timeouts on health check commands.

### osmoda-routines — Background Automation

- **Socket**: `/run/osmoda/routines.sock` (permissions: 0660)
- **State**: `/var/lib/osmoda/routines/`
- **Role**: Cron/interval scheduler for background tasks.
- **Default routines**: Health check (5m), service monitor (10m), log scan (15m) — matches HEARTBEAT.md cadences.
- **Triggers**: Cron expressions (`*/5 * * * *`), fixed intervals, event-based.
- **Actions**: HealthCheck, ServiceMonitor, LogScan, MemoryMaintenance (M1+), Command, Webhook (needs egress).
- **Hardening**: Graceful shutdown, subprocess timeouts on action execution.

## Data Flow

1. **User sends message** via web chat, Telegram, or WhatsApp → OpenClaw Gateway
2. **Gateway routes to single conversation** — all channels share one thread
3. **Prompt assembled** with system context
4. **Claude API call** via API key
5. **Claude responds** with text + tool calls
6. **Tool execution** → osmoda-bridge → daemon over Unix socket → structured JSON
7. **Results sent back** to Claude for synthesis
8. **Ledger event** created for any system mutation
9. **Response delivered** to originating channel
10. **Other channels notified** — web UI, Telegram, WhatsApp all see the same thread

## Chat Sync Model

One OS instance = one conversation. Multiple channels are windows into the same thread.

```
         Web UI (WebSocket)  ──┐
         Telegram (Bot API)  ──┤──→ OpenClaw Gateway ──→ Single Conversation
         WhatsApp (Baileys)  ──┘         │
                                         ▼
                                   agentd ledger
                                   (every message logged
                                    with channel source)
```

**How sync works:**
- OpenClaw gateway maintains one persistent conversation
- Incoming message from any channel → processed by the AI
- Response → delivered back to the channel that sent the message
- All channels see the full conversation history
- The agentd ledger stores every message as an event with `actor` field indicating the source channel

**Channel-aware responses:**
- The AI knows which channel the message came from
- Telegram/WhatsApp → shorter, punchier responses (user is on phone)
- Web chat → full detail, code blocks, verbose explanations

**Setup via conversation:**
Users don't edit config files. They tell the AI "connect Telegram" and the AI does it using its existing tools (`file_write` + `shell_exec`). The AI saves credentials, configures OpenClaw, restarts the gateway.

## Event Ledger

Every system mutation creates a hash-chained event:

```sql
CREATE TABLE events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  type TEXT NOT NULL,
  actor TEXT NOT NULL,
  payload TEXT NOT NULL,
  prev_hash TEXT NOT NULL,
  hash TEXT NOT NULL
);
```

```
hash = SHA-256(id|ts|type|actor|payload|prev_hash)   # pipe-delimited
```

Genesis event has `prev_hash` = all zeros. Chain is verifiable with `agentctl verify-ledger`.

## NixOS Integration

osModa is a NixOS module (`services.osmoda`). One `enable = true` activates:

- agentd systemd service (root, watchdog)
- OpenClaw Gateway systemd service (depends on agentd)
- osmoda-keyd service (PrivateNetwork, RestrictAddressFamilies)
- osmoda-watch service (root, for nixos-rebuild access)
- osmoda-routines service (systemd hardening)
- Egress proxy (DynamicUser, domain-filtered)
- Messaging channels (Telegram, WhatsApp — env vars injected into gateway)
- Workspace activation (skills, templates)
- Firewall defaults (nothing exposed)

NixOS provides atomic, rollbackable system changes — the safest OS for an AI to manage.

## Messaging Channels

OpenClaw supports Telegram and WhatsApp as messaging channels. osModa surfaces these as NixOS options that generate an OpenClaw config file.

```nix
services.osmoda.channels.telegram.enable = true;
services.osmoda.channels.telegram.botTokenFile = "/var/lib/osmoda/secrets/telegram-bot-token";
services.osmoda.channels.telegram.allowedUsers = [ "admin" ];

services.osmoda.channels.whatsapp.enable = true;
services.osmoda.channels.whatsapp.allowedNumbers = [ "+1234567890" ];
```

**How it works:**
1. NixOS module generates an OpenClaw config JSON from channel options
2. Config file is passed to the gateway via `--config`
3. OpenClaw reads the config and initializes its channel adapters
4. Telegram: bot token read from file, connects via Telegram Bot API
5. WhatsApp: uses Baileys for Web API, auth state stored in credential directory

**Important:** The actual channel implementation lives in OpenClaw, not in osModa. osModa provides the NixOS config surface and credential management. If OpenClaw's config format changes, the generated config file may need updating.

**Security:**
- Bot tokens stored in files with 0600 permissions, never in Nix config
- WhatsApp credentials in dedicated directory with 0700 permissions
- Allowlists prevent unauthorized access

## Memory Architecture (M0)

M0 uses ledger-based storage only. ZVEC vector search is designed but not yet wired.

```
User message → OpenClaw → Memory Backend search()
                              │
                              ├─ Embed query (local nomic model, 768-dim)  [M1+]
                              ├─ ZVEC semantic search                       [M1+]
                              ├─ SQLite FTS5 BM25 keyword search           [M1+]
                              ├─ RRF hybrid merge                          [M1+]
                              └─ Returns empty in M0

Ground truth: Markdown files at /var/lib/osmoda/memory/
ZVEC indexes are derived and always rebuildable.
```

## Provisioning Layer (spawn.os.moda)

spawn.os.moda is the commercial provisioning service (separate private repo). It handles payment, server creation, and ongoing management.

```
┌──────────────────────────────────────────────────────────┐
│  spawn.os.moda (Node.js + Express)                        │
│                                                            │
│  Landing page → USDC payment → Hetzner API → cloud-init   │
│  Management dashboard → status API → heartbeat receiver    │
└──────────┬───────────────────────────────────┬────────────┘
           │ creates server                     │ receives heartbeats
           ▼                                    ▲
┌──────────────────────────────────────────────────────────┐
│  osModa Server (provisioned)                               │
│                                                            │
│  install.sh --order-id UUID --callback-url URL             │
│  Stores config in /var/lib/osmoda/config/                  │
│  osmoda-heartbeat.timer → POST /api/heartbeat (5 min)     │
│  Sends: order_id, status, cpu, ram, disk, openclaw_ready  │
└──────────────────────────────────────────────────────────┘
```

**Auth model:** Order UUID = auth token. 128-bit unguessable. No passwords, no OAuth, no sessions. Rate-limited.

**Management dashboard:** User visits `/manage?id=UUID` → sees server status, health metrics, SSH/tunnel commands, upsell grid. Auto-refreshes every 30 seconds.

## Security Model

- **agentd**: Runs as root. This is intentional — it IS the system interface.
- **keyd**: Network-isolated. Keys encrypted at rest. Policy-gated signing. Zeroizes key material on drop.
- **watch**: Runs as root (needs `nixos-rebuild` and `systemctl` access). Auto-rollback is a safety net, not a security boundary.
- **routines**: systemd hardening (NoNewPrivileges, ProtectKernelTunables). Runs scheduled tasks only.
- **egress**: Domain allowlist. Only Ring 2 tools route through it.
- **All sockets**: Unix domain sockets with restrictive file permissions. No TCP between components.
