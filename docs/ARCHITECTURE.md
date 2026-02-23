# osModa Architecture

## Overview

osModa is a NixOS distribution where the AI agent IS the operating system interface. The agent has root access through `agentd`, a Rust daemon that provides structured, audited access to every aspect of the Linux system. Additional daemons handle crypto wallets (keyd), deploy transactions (watch), background automation (routines), and P2P encrypted server-to-server communication (mesh).

Local inter-daemon communication happens over Unix sockets. osmoda-mesh adds TCP port 18800 for peer-to-peer connections between osModa instances.

## Trust Rings

```
RING 0: OpenClaw + agentd + keyd + watch + routines + mesh
  Full system access. Root-equivalent. See and control everything.
  Components: OpenClaw Gateway, osmoda-bridge, agentd, keyd, watch, routines, mesh

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
│   osmoda-bridge plugin → 50 tools registered                 │
│   Memory Backend → ZVEC search → injects into prompt (M1+)  │
└──────┬──────────┬───────────┬──────────┬──────────┬──────────┘
       │          │           │          │          │
       ▼          ▼           ▼          ▼          ▼
┌──────────┐ ┌──────────┐ ┌─────────┐ ┌─────────┐ ┌─────────┐
│ agentd   │ │ keyd     │ │ watch   │ │routines │ │  mesh   │
│          │ │          │ │         │ │         │ │         │
│ /health  │ │ /wallet/ │ │/switch/ │ │/routine/│ │/invite/ │
│ /system/ │ │  create  │ │ begin   │ │ add     │ │ create  │
│ /events/ │ │  list    │ │ status  │ │ list    │ │ accept  │
│ /memory/ │ │  sign    │ │ commit  │ │ trigger │ │/peers   │
│ /agent/  │ │  send    │ │rollback │ │ history │ │/peer/{} │
│ /receipt │ │          │ │/watcher/│ │         │ │  send   │
│ /incidnt │ │ Private  │ │ add     │ │Scheduler│ │/identity│
│          │ │ Network  │ │ list    │ │ loop    │ │         │
│ Ledger   │ │ (no net) │ │ remove  │ │ every   │ │Noise_XX │
│ (SQLite) │ │          │ │         │ │  60s    │ │ML-KEM   │
│          │ │ AES-256  │ │ Health  │ │         │ │ 768     │
│ Hash     │ │ GCM keys │ │ checks  │ │ Cron +  │ │         │
│ chain    │ │          │ │ + auto  │ │interval │ │TCP:     │
│          │ │ Policy   │ │rollback │ │triggers │ │18800    │
│          │ │ engine   │ │         │ │         │ │         │
└──────────┘ └──────────┘ └─────────┘ └─────────┘ └─────────┘
  agentd.sock  keyd.sock  watch.sock routines.sock mesh.sock
  (root)       (root,     (root)      (root)       (root)
               no network)

All Unix sockets under /run/osmoda/
mesh also listens on TCP :18800 for peer-to-peer connections
```

## Daemon Details

### agentd — Kernel Bridge

- **Socket**: `/run/osmoda/agentd.sock`
- **State**: `/var/lib/osmoda/`
- **Role**: Central daemon. Provides system queries, audit ledger, memory endpoints, Agent Card (EIP-8004), receipts, and incident workspaces.
- **Ledger**: Append-only SQLite with SHA-256 hash chaining (pipe-delimited format). Every event references the previous hash. Chain verifiable with `agentctl verify-ledger`.
- **FTS5**: Full-text search index over all events with Porter stemming and BM25 ranking. Auto-synced via trigger on insert. Powers `memory/recall`.
- **Service Discovery**: `GET /system/discover` — parses `ss -tlnp` and `systemctl list-units` to find all running services, listening ports, and systemd units. Detects known service types (nginx, postgres, redis, node, etc.).
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

### osmoda-mesh — P2P Encrypted Communication

- **Socket**: `/run/osmoda/mesh.sock` (Unix, local API)
- **Port**: 18800/TCP (peer connections from other osModa instances)
- **State**: `/var/lib/osmoda/mesh/` (0700 permissions)
- **Role**: Enables encrypted, post-quantum-safe P2P communication between osModa instances.
- **Cipher suite**: Noise_XX (X25519/ChaChaPoly/BLAKE2s) for the handshake. After transport is established, ML-KEM-768 encapsulation happens inside the encrypted tunnel. Final session keys are re-derived by combining both shared secrets via `HKDF-SHA256`. If classical crypto breaks, ML-KEM protects. If ML-KEM breaks, classical protects.
- **Identity**: Each instance has a stable Ed25519 signing key, X25519 static key (for Noise), and ML-KEM-768 keypair. The `instance_id` is derived as `hex(SHA-256(noise_static_pubkey))[..32]`. Identity is signed so peers can verify authenticity.
- **Pairing**: Invite-based. No central registry. Initiating peer generates a base64url invite code (endpoint + public keys, TTL-limited). Accepting peer decodes the invite, connects, runs the full handshake.
- **Messages**: 10 typed variants (Heartbeat, HealthReport, Alert, Chat, LedgerSync, Command, CommandResponse, PeerAnnounce, KeyRotation, PqExchange). All encrypted on the wire.
- **Hardening**: Keys zeroized on Drop, all key files 0600, graceful shutdown with CancellationToken across all background loops.

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
- Remote access (Cloudflare Tunnel, Tailscale — optional)
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

## Remote Access

osModa supports two remote access methods, both configured as NixOS options:

### Cloudflare Tunnel
Exposes the gateway through Cloudflare's network. Quick tunnel mode requires no account — just `enable = true` and you get a random trycloudflare.com URL. For production, use your own tunnel with credentials.

```nix
services.osmoda.remoteAccess.cloudflare.enable = true;
# Optional: own tunnel
services.osmoda.remoteAccess.cloudflare.credentialFile = "/var/lib/osmoda/secrets/cf-creds.json";
services.osmoda.remoteAccess.cloudflare.tunnelId = "abc123";
```

### Tailscale
Joins the server to your Tailscale network. With an auth key file, login is automatic and headless.

```nix
services.osmoda.remoteAccess.tailscale.enable = true;
services.osmoda.remoteAccess.tailscale.authKeyFile = "/var/lib/osmoda/secrets/tailscale-key";
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

M0 uses ledger-based storage with FTS5 full-text search. ZVEC vector search is designed but not yet wired.

```
User message → OpenClaw → Memory Backend search()
                              │
                              ├─ SQLite FTS5 BM25 keyword search           [LIVE]
                              │   Porter stemming, unicode tokenization
                              │   Falls back to keyword scan if FTS5 fails
                              ├─ Embed query (local nomic model, 768-dim)  [M1+]
                              ├─ ZVEC semantic search                       [M1+]
                              └─ RRF hybrid merge                          [M1+]

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
- **mesh**: Noise_XX + ML-KEM-768 hybrid PQ. All peer traffic encrypted. Invite-based pairing, no global registry. Keys zeroized on shutdown.
- **Local sockets**: Unix domain sockets with restrictive file permissions (0600/0660).
- **Mesh TCP**: Port 18800. All traffic encrypted (Noise_XX transport mode). No plaintext ever on the wire.
