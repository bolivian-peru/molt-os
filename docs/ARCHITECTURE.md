# osModa Architecture

## Overview

osModa is a NixOS distribution with AI-native system management. The agent has root access through `agentd`, a Rust daemon that provides structured, audited access to every aspect of the Linux system. Additional daemons handle crypto wallets (keyd), deploy transactions (watch), background automation (routines), P2P encrypted server-to-server communication (mesh), local voice (voice), MCP server lifecycle management (mcpd), and system learning & self-optimization (teachd).

Local inter-daemon communication happens over Unix sockets. osmoda-mesh adds TCP port 18800 for peer-to-peer connections between osModa instances.

> **Runtime requirement:** osModa requires a real NixOS system (bare metal, cloud VM, or QEMU/KVM). It cannot run inside Docker, LXC, WSL, or any container runtime — the daemons need systemd, NixOS package management, and full kernel access.

## Trust Tiers

```
TIER 0: osmoda-gateway + agentd + keyd + watch + routines + mesh + voice + mcpd + teachd
  Full system access. Root-equivalent. See and control everything.
  Components: osmoda-gateway (modular), driver-of-choice (claude-code or openclaw),
  osmoda-bridge OR osmoda-mcp-bridge (91 tools), plus all Rust daemons above.

TIER 1: Approved Apps
  Sandboxed with declared capabilities. No root, no arbitrary filesystem.
  Execution: bubblewrap + systemd transient units
  Network: egress proxy with domain allowlist

TIER 2: Untrusted Execution
  Maximum isolation. Working directory + /tmp only. No network.
  User scripts, pip packages, npm installs, third-party binaries.
```

## Agent gateway — modular runtime (v0.2+)

```
                      ┌─────────────────────────┐
                      │   osmoda-gateway        │
                      │   (TypeScript)          │
                      │   systemd: always this  │
                      └────────────┬────────────┘
                                   │ reads
                                   ▼
                ┌──────────────────────────────────────┐
                │ /var/lib/osmoda/config/agents.json   │
                │   { agents: [{ id, runtime, cred, …}]}│
                │ /var/lib/osmoda/config/credentials.*  │
                │   AES-256-GCM encrypted store         │
                └──────┬──────────────────┬────────────┘
                       │                  │
              route per-agent       decrypt on use
                       │                  │
              ┌────────┴──────┐    ┌──────┴───────┐
              ▼               ▼    ▼              ▼
      ┌──────────────┐  ┌──────────────┐  ┌────────────────┐
      │ claude-code  │  │ openclaw     │  │ codex (future) │
      │ driver       │  │ driver       │  │ driver         │
      ├──────────────┤  ├──────────────┤  ├────────────────┤
      │ spawns       │  │ spawns       │  │ …              │
      │ `claude` CLI │  │ `openclaw`   │  │                │
      │ --bare OR    │  │ binary as    │  │                │
      │ OAuth env    │  │ child proc   │  │                │
      └──────┬───────┘  └──────┬───────┘  └────────────────┘
             │                 │
             └────────┬────────┘
                      ▼
              ┌───────────────┐
              │ MCP bridge    │   91 tools, runtime-neutral
              │ (91 tools)    │
              └───────┬───────┘
                      ▼
        (all osModa daemons via Unix sockets)
```

**Per-session routing:** WS `/ws` or POST `/telegram` → gateway looks up agent
for channel → fetches credential → invokes driver → streams `AgentEvent` back.

**Hot-reload:** `kill -HUP <pid>` (or `systemctl reload osmoda-gateway`) re-reads
agents.json. In-flight sessions keep their original driver + credential snapshot.

**REST config API** (Bearer-authed): `/config/drivers`, `/config/agents`,
`/config/credentials`, `/config/credentials/:id/test`, `/config/reload`,
`/config/health`. Used by the dashboard to switch runtime / swap credentials
/ test validity without SSH.

## Component Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ User (Terminal / Browser / Chat)                             │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│ osmoda-gateway (:18789) — modular, TypeScript                 │
│   reads agents.json → routes per-agent to a driver            │
│   drivers: claude-code (default) · openclaw (legacy)          │
│   osmoda-mcp-bridge → 91 MCP tools                            │
│   Memory Backend → FTS5 BM25 search (live) · vector (M1+)   │
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

### agentd — System Bridge

- **Socket**: `/run/osmoda/agentd.sock`
- **State**: `/var/lib/osmoda/`
- **Role**: Central daemon. Provides system queries, audit ledger, memory endpoints, Agent Card (EIP-8004), receipts, and incident workspaces.
- **Ledger**: Append-only SQLite with SHA-256 hash chaining (pipe-delimited format). Every event references the previous hash. Chain verifiable with `agentctl verify-ledger`.
- **FTS5**: Full-text search index over all events with Porter stemming and BM25 ranking. Auto-synced via trigger on insert. Powers `memory/recall`.
- **Service Discovery**: `GET /system/discover` — parses `ss -tlnp` and `systemctl list-units` to find all running services, listening ports, and systemd units. Detects known service types (nginx, postgres, redis, node, etc.).
- **Backup**: Daily systemd timer backs up SQLite state with WAL checkpointing. 7-day retention with automatic cleanup.
- **Hardening**: Graceful shutdown (SIGTERM/SIGINT), subprocess timeout protection, input validation with path traversal rejection.

### osmoda-keyd — Crypto Wallets (Optional)

Optional module for AI agent workloads that need cryptographic signing. Not required for core system management.

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

### osmoda-voice — Local Voice Pipeline

- **Socket**: `/run/osmoda/voice.sock`
- **State**: `/var/lib/osmoda/voice/`
- **Role**: 100% local speech-to-text and text-to-speech. No cloud APIs, no data leaves the machine.
- **STT**: whisper.cpp (MIT license), configurable model, 16kHz mono WAV input.
- **TTS**: piper-tts (MIT license), ONNX models, audio output via PipeWire.
- **Hardening**: Graceful shutdown, subprocess timeouts on model inference.

### osmoda-mcpd — MCP Server Manager

- **Socket**: `/run/osmoda/mcpd.sock`
- **State**: `/var/lib/osmoda/mcp/`
- **Role**: Lifecycle manager for MCP (Model Context Protocol) servers. Starts, monitors, restarts MCP server processes. Generates OpenClaw MCP config from NixOS options.
- **Not a proxy**: OpenClaw connects to MCP servers directly via stdio. mcpd manages the process lifecycle only.
- **Security**: Servers with `allowedDomains` get `HTTP_PROXY` injected to route traffic through osmoda-egress. Secret files injected as env vars.
- **Monitoring**: 10-second health check loop detects crashed servers and auto-restarts.
- **Audit**: All server lifecycle events (start, stop, crash, restart) logged to agentd ledger.
- **NixOS config**: Declare MCP servers in `services.osmoda.mcp.servers` and they become available to the AI.

### osmoda-teachd — System Learning & Self-Optimization

- **Socket**: `/run/osmoda/teachd.sock`
- **State**: `/var/lib/osmoda/teachd/`
- **Role**: Continuously observes the system, detects patterns, generates reusable knowledge documents, suggests/applies optimizations, and auto-generates skills from repeated agent behavior. When OpenClaw troubleshoots a problem, the knowledge persists as reusable system wisdom. When the agent repeats the same tool sequence across multiple sessions, teachd creates a reusable skill automatically.

**Four-loop architecture:**

```
OBSERVE (30s)          LEARN (5m)            SKILLGEN (1h)         TEACH (on-demand)
─────────────          ──────────            ─────────────         ─────────────────
/proc/stat      ──┐    Recurring failures    Agent actions  ──┐    POST /teach
/proc/meminfo   ──┤    Resource trends       (tool sequences)│    → keyword match
systemctl       ──┼──→ Anomaly detection     Sequence detect ┼──→ → relevant docs
journalctl      ──┘    Correlations          Dedup + score   │    → token budget
     │                      │                Generate SKILL  ──┘
     ▼                      ▼                      │
Observations           Patterns → Knowledge    SKILL.md files
(SQLite, 7d TTL)       (confidence > 0.7)      (auto-generated)
                             │
                             ▼
                       Optimizations (via SafeSwitch)
```

See [SKILL-LEARNING.md](SKILL-LEARNING.md) for the full skill auto-teaching pipeline.

**Data flow:**
1. **OBSERVE** loop collects CPU, memory, service states, and journal errors every 30 seconds
2. **LEARN** loop analyzes recent observations every 5 minutes, detecting:
   - **Recurring failures**: same service/identifier failing 3+ times
   - **Resource trends**: monotonic increase in memory/CPU over 1 hour
   - **Anomalies**: sudden spikes (>2 std deviations from rolling average)
   - **Correlations**: events occurring within 60s of each other (e.g., high CPU + service crash)
3. High-confidence patterns (>0.7) generate **Knowledge Documents** (markdown, categorized, tagged)
4. **TEACH** API matches a context query against knowledge docs using keyword scoring + confidence weighting
5. **Optimizer** generates suggestions from unapplied knowledge, applies changes via SafeSwitch

**Integration points:**
- **agentd**: Receipt logging for all teach events (pattern detection, knowledge creation, optimization)
- **osmoda-watch**: SafeSwitch sessions for applying optimizations safely with auto-rollback
- **osmoda-bridge**: 13 tools expose teachd capabilities to OpenClaw (8 core + 5 skill learning)

- **Hardening**: Graceful shutdown with CancellationToken, subprocess timeouts on systemctl/sysctl calls, 7-day observation pruning.

### App Management — Bridge Tools

- **No daemon**: App management runs entirely through osmoda-bridge (6 tools), not a separate Rust daemon.
- **Mechanism**: `systemd-run` creates transient systemd services. Each app gets its own cgroup, journal log stream, and optional resource limits (MemoryMax, CPUQuota).
- **Isolation**: `DynamicUser=yes` by default — ephemeral UID per app, no root. Optional `user` parameter for apps that need filesystem access.
- **Registry**: JSON file at `/var/lib/osmoda/apps/registry.json`. Atomic writes (write `.tmp` then `rename`).
- **Boot persistence**: `osmoda-app-restore.service` (oneshot) reads the registry on boot and re-creates transient units for all apps marked as `running`.
- **Audit**: All deploy/stop/restart/remove operations logged to agentd ledger via `/memory/ingest`.

## Multi-Agent Routing

One `osmoda-gateway`, multiple routed agents. Each agent is an isolated config entry in
`/var/lib/osmoda/config/agents.json` with its own runtime, credential, model, workspace,
session store, and channel bindings. Agents can share or diverge on any of those axes
independently.

```
                    ┌─────────────────────────────────┐
                    │   osmoda-gateway (:18789)        │
                    │   reads agents.json → routes     │
                    └──────┬──────────────┬────────────┘
                           │              │
              ┌────────────▼──┐    ┌──────▼────────────┐
              │  osmoda agent  │    │   mobile agent     │
              │  runtime:      │    │   runtime:         │
              │   claude-code  │    │    claude-code     │
              │  Opus 4.6      │    │   Sonnet 4.6       │
              │  91 tools      │    │   91 tools         │
              │  19 skills     │    │   19 skills        │
              │  Full access   │    │   Full access      │
              │                │    │   Concise replies  │
              │  ← Web chat    │    │   ← Telegram       │
              │                │    │   ← WhatsApp       │
              └────────────────┘    └────────────────────┘
```

**Default agents** (install.sh writes both; they point to the same credential by default):

| Agent | Model | Tools | Skills | Channels |
|-------|-------|-------|--------|----------|
| `osmoda` (default) | claude-opus-4-6 | All 91 | All 19 | Web chat (default) |
| `mobile` | claude-sonnet-4-6 | All 91 | All 19 | Telegram, WhatsApp |

Any agent's `runtime` can be flipped to `openclaw` from the dashboard Engine tab or via
`PATCH /config/agents/:id`. The systemd unit does not change — `osmoda-gateway` simply picks
the `openclaw` driver instead of the `claude-code` driver for that agent's next session.

**Routing rules:** Bindings route Telegram and WhatsApp to `mobile`. Everything else (web chat) falls through to `osmoda` (marked as `default: true`).

**Per-agent workspaces:**
- `/var/lib/osmoda/workspace-osmoda/` — Full AGENTS.md, SOUL.md, TOOLS.md, HEARTBEAT.md, all skills
- `/var/lib/osmoda/workspace-mobile/` — Mobile-optimized AGENTS.md + SOUL.md (concise style), all skills

**Tool access:** Both agents have full access to all 90 tools. The mobile agent differs only in response style (concise, phone-optimized) and model (Sonnet for faster responses on mobile).

## Data Flow

1. **User sends message** via web chat, Telegram, or WhatsApp → osmoda-gateway
2. **Gateway routes to agent** — bindings match channel → agent (mobile for Telegram/WhatsApp, osmoda for web)
3. **Agent workspace loaded** — per-agent AGENTS.md, SOUL.md, skills
4. **Prompt assembled** with agent-specific system context
5. **Driver invoked** — claude-code or openclaw, per the agent's `runtime` field
6. **LLM call** via the agent's `credential` (Anthropic OAuth, API key, or OpenAI/OpenRouter)
7. **Tool execution** → MCP bridge (or osmoda-bridge for the openclaw driver) → daemon over Unix socket → structured JSON
8. **Results sent back** to Claude for synthesis
9. **Ledger event** created for any system mutation
10. **Response delivered** to originating channel

## Chat Sync Model

One OS instance = one conversation. Multiple channels are windows into the same thread.

```
         Web UI (WebSocket)  ──┐
         Telegram (Bot API)  ──┤──→ osmoda-gateway ──→ Single Conversation
         WhatsApp (Baileys)  ──┘         │
                                         ▼
                                   agentd ledger
                                   (every message logged
                                    with channel source)
```

**How sync works:**
- osmoda-gateway maintains one persistent conversation
- Incoming message from any channel → processed by the AI
- Response → delivered back to the channel that sent the message
- All channels see the full conversation history
- The agentd ledger stores every message as an event with `actor` field indicating the source channel

**Channel-aware responses:**
- The AI knows which channel the message came from
- Telegram/WhatsApp → shorter, punchier responses (user is on phone)
- Web chat → full detail, code blocks, verbose explanations

**Setup via conversation:**
Users don't edit config files directly. They tell the agent "connect Telegram" and it does the
work using `file_write` + `shell_exec`: saves the bot token, edits NixOS config, runs
`nixos-rebuild switch`. The modern way is to use the dashboard's **Engine** tab — no SSH and no
nixos-rebuild needed for credential/runtime/model changes, only for new channel setup.

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
- osmoda-gateway systemd service (depends on agentd)
- osmoda-keyd service (PrivateNetwork, RestrictAddressFamilies)
- osmoda-watch service (root, for nixos-rebuild access)
- osmoda-routines service (systemd hardening)
- osmoda-voice service (requires PipeWire for audio I/O)
- osmoda-mcpd service (MCP server lifecycle, depends on agentd + egress)
- osmoda-teachd service (system learning, depends on agentd)
- Egress proxy (DynamicUser, domain-filtered)
- Messaging channels (Telegram, WhatsApp — env vars injected into gateway)
- Remote access (Cloudflare Tunnel, Tailscale — optional)
- Workspace activation (skills, templates)
- Firewall defaults (nothing exposed)

NixOS provides atomic, rollbackable system changes — the safest OS for an AI to manage.

## Messaging Channels

`osmoda-gateway` has first-class Telegram support (webhook at `POST /telegram`). WhatsApp
is available via an MCP server managed by `osmoda-mcpd`. Channels route to agents through
`agents.json` bindings — see [CHANNELS.md](CHANNELS.md).

```nix
services.osmoda.gateway.telegram.enable = true;
services.osmoda.gateway.telegram.tokenFile = "/var/lib/osmoda/secrets/telegram-bot-token";
services.osmoda.gateway.telegram.allowedUsers = [ "admin_username" ];
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
1. NixOS module writes `/var/lib/osmoda/config/agents.json` with bindings `[{channel, agent_id}]`
2. `osmoda-gateway` reads that on startup and on `SIGHUP`
3. Telegram: bot token read from `tokenFile`, webhook at `POST /telegram` consumes updates
4. WhatsApp: MCP server managed by `osmoda-mcpd` handles the Baileys device-pairing flow
5. Each inbound message → gateway looks up the bound agent → loads its credential → invokes its driver → replies

**Important:** Channel implementation lives in `osmoda-gateway` (for Telegram) and in MCP
servers (for WhatsApp). osModa provides the NixOS config surface + `agents.json` routing.

**Security:**
- Bot tokens stored in files with 0600 permissions, never in Nix config
- WhatsApp credentials in dedicated directory with 0700 permissions
- Allowlists prevent unauthorized access

## Memory Architecture (M0)

M0 uses ledger-based storage with FTS5 full-text search. Semantic vector search (via usearch + fastembed) is designed but deferred to M1.

```
User message → osmoda-gateway → Memory Backend search()
                              │
                              ├─ SQLite FTS5 BM25 keyword search           [LIVE]
                              │   Porter stemming, unicode tokenization
                              │   Falls back to keyword scan if FTS5 fails
                              ├─ Embed query (local nomic model, 768-dim)  [M1+]
                              ├─ usearch semantic vector search              [M1+]
                              └─ RRF hybrid merge                          [M1+]

Ground truth: Markdown files at /var/lib/osmoda/memory/
Vector indexes (when wired) are derived and always rebuildable.
```

## Hosted Provisioning (spawn.os.moda)

spawn.os.moda is the hosted option for deploying osModa servers. Handles payment, server creation, and ongoing management via a web dashboard. Separate private repo — not part of the open source OS.

**Server detail dashboard:** Single-column tabbed layout (Overview / Chat / Settings). Overview tab shows agent card, orchestration cards (automation, activity feed, intelligence, tool servers), channel cards (Telegram blue / WhatsApp green), system + settings grid, and collapsible setup progress. Chat tab has a horizontal activity bar, Claude-like rounded input, markdown rendering (code blocks, lists, headers, links, blockquotes), no-bubble agent messages, and user messages as accent bubbles.

**Orchestration dashboard cards** (real daemon data, not mocks):
- **Automation** — Active routines with intervals and last-run times, health watchers with check types and status (from routines + watch daemons)
- **Activity Feed** — 15 most recent agentd audit log events with timestamps, types, and actors
- **System Intelligence** — TeachD observation/pattern/knowledge counts, detected patterns with confidence scores
- **Tool Servers** — Running MCP servers with status, PID, and uptime

**Heartbeat data pipeline:** 60-second heartbeat from each server collects daemon data via Unix sockets (routines, watchers, events, teachd health/patterns, MCP servers, SafeSwitch sessions, NixOS generation) and sends to spawn for dashboard rendering.

### v1 Programmatic API (x402 payment-gated)

Agents can spawn osModa servers programmatically — agents spawning agents.

**Discovery:**
- `GET /.well-known/agent-card.json` — A2A/ERC-8004 Agent Card with plans as skills, x402 pricing, input/output schemas

**Endpoints:**
- `GET /api/v1/plans` — Plan list with x402 pricing info, regions, network mode (free)
- `POST /api/v1/spawn/:planId` — Spawn a server (x402-gated with USDC). Returns order ID + API token (`osk_...`)
- `GET /api/v1/status/:orderId` — Server status (basic info free, full details require Bearer `osk_` token)
- `WS /api/v1/chat/:orderId` — WebSocket chat with the server's AI agent (auth via `?token=osk_...`)
- `GET /api/v1/docs` — OpenAPI 3.0 spec with x402 extensions

**Payment:** Coinbase x402 protocol. USDC on Base (EVM) or Solana (SVM). Per-plan static pricing. Uses `@x402/express` + `@x402/evm` + `@x402/svm` + `@x402/core`.

**Auth:** API tokens (`osk_...`) are cryptographically random (32 bytes), returned once at spawn. Stored server-side as SHA-256 hash, compared with timing-safe equality. Bearer token auth for status + WebSocket chat.

**Agent skill doc:** `GET /SKILL.md` — 369-line plain-text document for agents to read. Covers full API, x402 flow, all 90 tools, mesh networking, self-install fallback.

## Safety Boundaries

### What's enforced today

| Protection | Implementation | Verified |
|-----------|---------------|----------|
| **Hash-chained audit ledger** | SHA-256 chain in SQLite, every mutation logged, verifiable with `agentctl verify-ledger` | 321+ events, zero broken links |
| **SafeSwitch deploys** | Health checks + TTL + auto-rollback on failure via osmoda-watch | Functional, tested |
| **Command blocklist** | 17 dangerous patterns blocked in shell_exec (rm -rf, dd, mkfs, etc.) | Pentest verified |
| **Rate limiting** | shell_exec 30/60s, mesh TCP 5/60s, file_read 10 MiB cap | Pentest verified |
| **Socket permissions** | All sockets 0600, all 9 daemons enforce umask(0o077) at startup | Pentest verified |
| **Input validation** | Path traversal rejection, symlink escape prevention, payload size limits, arg metachar rejection | Pentest verified |
| **Safety commands** | safety_rollback/panic/status/restart bypass the AI entirely | Functional |
| **NixOS atomicity** | Every system change is a generation, rollback is one command | Core NixOS feature |
| **Pentest results** | SQL injection, path traversal, shell injection, payload bombs, error hardening, stress testing | All pass (2026-02-27) |

### What's planned but not yet implemented

| Feature | Status | Why it matters |
|---------|--------|---------------|
| **Approval gate for destructive ops** | Planned (#1 priority) | Currently convention-based (agent prompt says "ask before destructive actions") — not enforced by code |
| **Tier 1/Tier 2 sandbox** | Designed, not enforced | Third-party tools should run in bubblewrap isolation with egress proxy, but this isn't wired yet |
| **Capability tokens** | Planned | Fine-grained, time-limited access tokens for socket auth; currently file-permissions only |
| **External security audit** | Planned | Mesh crypto uses standard primitives (Noise_XX, ML-KEM-768) but needs independent review |

## Security Model

- **agentd**: Runs as root. This is intentional — it IS the system interface.
- **keyd**: Network-isolated. Keys encrypted at rest. Policy-gated signing. Zeroizes key material on drop.
- **watch**: Runs as root (needs `nixos-rebuild` and `systemctl` access). Auto-rollback is a safety net, not a security boundary.
- **routines**: systemd hardening (NoNewPrivileges, ProtectKernelTunables). Runs scheduled tasks only.
- **egress**: Domain allowlist. Only Tier 2 tools route through it.
- **mesh**: Noise_XX + ML-KEM-768 hybrid PQ. All peer traffic encrypted. Invite-based pairing, no global registry. Keys zeroized on shutdown.
- **voice**: Local-only processing. No cloud APIs, no network calls. whisper.cpp + piper-tts, both MIT-licensed.
- **mcpd**: Lifecycle manager only — doesn't proxy MCP traffic. Injects egress proxy for domain-restricted servers. Secret files read from disk, never stored in Nix config.
- **teachd**: Read-only observation of system metrics. Optimizations require explicit approval flow (Suggested → Approved → Applied). Changes applied via SafeSwitch with auto-rollback safety net. 7-day observation TTL limits data retention.
- **Local sockets**: Unix domain sockets with restrictive file permissions (0600/0660).
- **Mesh TCP**: Port 18800. All traffic encrypted (Noise_XX transport mode). No plaintext ever on the wire.
