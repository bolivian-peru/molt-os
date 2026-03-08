<div align="center">

# osModa

### The first operating system built for AI agents.

**Your server has an AI brain. It monitors, fixes, deploys, and explains — without you SSH-ing in.**

9 Rust daemons. 88 structured tools. Tamper-proof audit ledger. Atomic rollback on every change. Post-quantum encrypted mesh between servers. Self-teaching skill engine that learns from agent behavior. All running on NixOS — the only Linux distro where every system state is a transaction.

> **Public Beta** — This is a working system deployed on real servers, not a demo. Expect rough edges and rapid iteration. You're early.

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Beta](https://img.shields.io/badge/Status-Public%20Beta-yellow.svg)]()
[![Rust](https://img.shields.io/badge/Rust-10%20crates-orange.svg)](https://www.rust-lang.org/)
[![NixOS](https://img.shields.io/badge/NixOS-Atomic-5277C3.svg)](https://nixos.org/)
[![Tests](https://img.shields.io/badge/Tests-205%20passing-brightgreen.svg)]()
[![Tools](https://img.shields.io/badge/Agent%20Tools-88-blueviolet.svg)]()

[Quickstart](#quickstart) · [Architecture](#architecture) · [What It Does](#what-it-does) · [Safety](#safety-model) · [API](#api-reference) · [Development](#development)

[![Telegram](https://img.shields.io/badge/Telegram-Join-blue?logo=telegram)](https://t.me/osmodasystems)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.gg/G7bwet8B)
[![Deploy](https://img.shields.io/badge/Deploy-spawn.os.moda-4f46e5.svg)](https://spawn.os.moda)

</div>

---

## Who Is This For

- **Solo developers who run their own servers** — your server monitors itself, fixes problems at 3am, and tells you about it in the morning
- **AI agent builders who need infrastructure** — deploy agents with API key management, health monitoring, crash recovery, and an encrypted mesh between machines
- **Small teams tired of on-call rotations** — the AI handles routine ops, escalates what it can't fix, and keeps a tamper-proof audit trail of everything it does
- **Anyone building an AI workforce** — osModa is the computer your agents live in. Not a VPS you SSH into. A machine that manages itself.

## Why This Exists

Every AI agent framework assumes your infrastructure is someone else's problem. They give you an agent that can think — but nowhere for it to live. So you SSH into a VPS, install things manually, pray nothing breaks at 3am, and when it does, you're the one waking up.

osModa is the other half: **the machine itself is AI-native.** 88 structured tools across 9 Rust daemons give the AI typed, auditable access to the entire operating system. No shell parsing. No regex. `system_health` returns structured JSON. Every mutation is SHA-256 hash-chained to a tamper-proof ledger. If a deploy breaks something, NixOS rolls back the entire system state atomically. If a service dies at 3am, the watcher detects it, the agent diagnoses root cause, SafeSwitch deploys a fix — with automatic rollback if health checks fail.

**Why NixOS?** Every system change is a transaction. Every state has a generation number. Rolling back is one command. This makes AI root access meaningfully safer than on any traditional Linux distribution. (NixOS rollback covers OS state — not data sent to external APIs or deleted user data. See [Safety Model](#safety-model).)

## What Happens in the First 5 Minutes

1. **Install** — add the flake to your NixOS config and `nixos-rebuild switch`
2. **Open the web chat** — the AI greets you and runs a health check on your server
3. **Ask "How's my server doing?"** — the AI calls `system_health` and `system_discover`, shows you what's running
4. **Say "Set up Telegram"** — it walks you through creating a bot and connecting it, so you can message your server from your phone
5. **Try something real** — "Install nginx and set up a reverse proxy for port 3000" — the AI edits NixOS config, rebuilds via SafeSwitch with auto-rollback if anything breaks

See the full [Getting Started Guide](docs/GETTING-STARTED.md) for a detailed walkthrough with expected output at each step.

## System Requirements

> **osModa is a full NixOS operating system, not an app.** It replaces your OS entirely — like installing Arch or Fedora, not like running `apt install`. It will NOT work inside Docker, LXC, or any container runtime. Containers lack systemd, NixOS package management, and the kernel-level access that osModa's 9 daemons require.

| Requirement | Details |
|------------|---------|
| **Platform** | Bare metal server, cloud VM (Hetzner, DigitalOcean, AWS), or QEMU/KVM virtual machine |
| **Architecture** | x86_64 or aarch64 |
| **RAM** | 2 GB minimum (4 GB recommended) |
| **Disk** | 20 GB minimum |
| **OS** | Fresh Ubuntu 22.04+, Debian 12+, or existing NixOS (installer converts to NixOS) |
| **NOT supported** | Docker, LXC, WSL, OpenVZ, or any container-based environment |

## Quickstart

### NixOS (flake) — recommended

```nix
# flake.nix
inputs.os-moda.url = "github:bolivian-peru/os-moda";

# configuration.nix
imports = [ os-moda.nixosModules.default ];
services.osmoda.enable = true;
```

```bash
sudo nixos-rebuild switch
curl -s --unix-socket /run/osmoda/agentd.sock http://localhost/health | jq
```

This is the primary install path. NixOS flakes give you reproducible builds, atomic upgrades, and instant rollback.

### Any Linux Server — experimental

> **Warning:** This converts your host OS to NixOS. It is a destructive, irreversible operation. Use on fresh/disposable servers only. Not recommended for production machines with existing workloads.

```bash
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash
```

Converts Ubuntu/Debian to NixOS, builds 10 Rust daemons from source, installs the AI gateway + 88 tools, starts everything. Takes ~10 minutes on a CX22.

**Supported:** Ubuntu 22.04+, Debian 12+, existing NixOS. x86_64 and aarch64.

### Deploy to Hetzner/DigitalOcean/AWS

```bash
git clone https://github.com/bolivian-peru/os-moda.git && cd os-moda
./scripts/deploy-hetzner.sh <server-ip> [ssh-key-path]
```

### Verify

```bash
# System health (structured JSON, not text parsing)
curl -s --unix-socket /run/osmoda/agentd.sock http://localhost/health | jq

# Audit ledger integrity
agentctl verify-ledger --state-dir /var/lib/osmoda
```

---

## Architecture

9 Rust daemons communicating over Unix sockets. No daemon exposes TCP to the internet (except mesh peer port 18800, encrypted). The AI reaches the system exclusively through structured tool calls, never raw shell. One gateway, multiple routed agents — Opus for deep work (web), Sonnet for quick status (mobile).

```
┌──────────────────────────────────────────────────────────────────────────────┐
│  User — Terminal / Web / Telegram / WhatsApp                                  │
├──────────────────────────────────────────────────────────────────────────────┤
│  AI Gateway (OpenClaw)          Multi-Agent Router                            │
│  ├─ osmoda agent (Opus)         88 tools · 17 skills · full access · web      │
│  └─ mobile agent (Sonnet)       full access · concise replies · Telegram/WA    │
│  osmoda-bridge                  88 typed tools (shared plugin)                 │
│  MCP Servers (stdio)            managed by osmoda-mcpd                        │
├────────┬────────┬────────┬──────────┬────────┬───────┬──────┬───────┬───────┤
│ agentd │ watch  │routine │ teachd   │ mesh   │ voice │ mcpd │ keyd  │egress │
│ System │ Safe   │ Cron + │ System   │ P2P    │ Local │ MCP  │Crypto │Domain │
│ bridge │ Switch │ event  │ learn    │Noise_XX│ STT/  │server│wallet │filter │
│ ledger │ roll-  │automate│ self-    │+ML-KEM │ TTS   │life- │ETH+   │proxy  │
│ memory │ back   │        │ optim    │hybrid  │       │cycle │SOL    │       │
├────────┴────────┴────────┴──────────┴────────┴───────┴──────┴───────┴───────┤
│  NixOS — atomic rebuilds · instant rollback · generations                     │
└──────────────────────────────────────────────────────────────────────────────┘
```

### Trust Model (3 tiers)

```
TIER 0  OpenClaw + agentd       Root. Full system. This is the agent.
TIER 1  Approved apps           Sandboxed. Declared capabilities only.
TIER 2  Untrusted tools         Max isolation. No network. Minimal filesystem.
```

The agent is tier 0 by design. It's not a chatbot with sudo — it's a system service with structured access to everything, constrained by NixOS atomicity and its own audit ledger, not by permission denials. Lower tiers cannot escalate privileges upward by design. Tier 0 remains the trusted computing base and must be governed by approval policies, spending limits, and audit review.

---

## Safety Model

The #1 question: "Why does the AI have root access?" Because it IS the system interface — the same way systemd has root access. The safety model is not about restricting the agent, but about making its actions auditable, reversible, and bounded.

### What protects you today

| Protection | How it works |
|-----------|-------------|
| **NixOS atomic rollback** | Every system change is a generation. Bad config? One command reverts the entire OS state. |
| **Hash-chained audit ledger** | Every action creates a SHA-256-chained event in SQLite. Tamper-evident. Verifiable offline with `agentctl verify-ledger`. 321+ events verified on live server with zero broken links. |
| **SafeSwitch deploys** | Changes go through a probation period with health checks. If any check fails, automatic rollback to the previous generation. |
| **Command blocklist** | 17 dangerous command patterns blocked in `shell_exec` (rm -rf, dd, mkfs, etc.). Expanded and pentest-verified. |
| **Rate limiting** | All public endpoints enforce rate limits (shell_exec: 30/60s, mesh TCP: 5/60s). |
| **Socket permissions** | All Unix sockets are 0600 (owner-only). All 9 daemons enforce `umask(0o077)` at startup. |
| **Approval gates** | Destructive operations require explicit approval via `approval_request`/`approval_approve`. Time-limited with auto-expiry. |
| **Fleet coordination** | Multi-server changes go through quorum voting via `fleet_propose`/`fleet_vote` before applying. |
| **Safety commands** | `safety_rollback`, `safety_panic`, `safety_status`, `safety_restart` bypass the AI entirely — the user always has an escape hatch. |
| **Pentest verified** | Full automated pentest: injection attacks (SQL, path traversal, shell), payload bombs, error hardening, stress testing (700/700 concurrent health checks). All pass. |

### What NixOS rollback covers — and what it doesn't

**Covered:** OS configuration, package state, service definitions, firewall rules, system generations. Any bad config change can be atomically reverted.

**NOT covered:** Data already sent to external APIs, signed crypto transactions, deleted user data, exposed secrets, or side effects on remote systems. Tier 0 access means the agent can do anything the system can do — the safety model relies on structured tools, audit trails, and NixOS atomicity, not on restricting the agent's access.

### What's planned but not yet complete

- **Tier 1/Tier 2 sandbox enforcement** — the trust tier model is designed and `sandbox_exec` exists, but bubblewrap isolation isn't fully wired for all third-party tools yet.
- **Capability token auth** — `capability_mint` can create time-limited tokens, but socket authentication is still primarily file-permissions based.
- **External security audit** — mesh crypto uses standard primitives (Noise_XX, ML-KEM-768) but hasn't had independent review.
- **Semantic memory** — `memory/recall` uses FTS5 BM25 keyword search. Semantic vector search (usearch + fastembed) is designed but not yet wired.

### Audit Ledger

Every mutation creates a hash-chained event in SQLite:

```
hash = SHA-256(id | ts | type | actor | payload | prev_hash)
```

Append-only. Tamper-evident. Any single modification invalidates the chain. Verifiable offline with `agentctl verify-ledger`.

---

## What It Does

### Core: System Management

| Daemon | What it does | Key feature |
|--------|-------------|-------------|
| **agentd** | System bridge: processes, services, network, filesystem, NixOS config, sysctl parameters. Hash-chained audit ledger. FTS5 memory search. | The structured interface between AI and system |
| **osmoda-watch** | SafeSwitch: deploy with a timer, health checks, and automatic rollback if anything fails. Autopilot watchers with escalation (restart -> rollback -> notify). | Blue-green deploys with automatic undo |
| **osmoda-routines** | Background cron/event/webhook automation. Runs between conversations. Health checks, log scans, service monitors. | Agent actions that persist when nobody's chatting |
| **osmoda-teachd** | OBSERVE loop (30s) collects metrics. LEARN loop (5m) detects patterns. SKILLGEN loop (1h) detects repeated agent tool sequences and auto-generates SKILL.md files. TEACH API injects knowledge. Optimizer suggests fixes. | The OS learns from its own behavior and teaches itself new skills |

### Communication

| Daemon | What it does | Key feature |
|--------|-------------|-------------|
| **osmoda-mesh** | P2P encrypted agent-to-agent communication. Noise_XX + ML-KEM-768 hybrid post-quantum. Invite-based pairing. | Servers talk to each other, end-to-end encrypted |
| **osmoda-voice** | Local speech-to-text (whisper.cpp) + text-to-speech (piper). All processing on-device. No cloud APIs. | Fully local voice, zero cloud dependency |

### Infrastructure

| Daemon | What it does | Key feature |
|--------|-------------|-------------|
| **osmoda-mcpd** | MCP server lifecycle manager. Starts, monitors, restarts MCP servers from NixOS config. | Any MCP server becomes an OS capability |
| **osmoda-egress** | HTTP CONNECT proxy with domain allowlist per capability token. | Sandboxed tools can't phone home |

### Optional: Crypto Wallet

| Daemon | What it does | Key feature |
|--------|-------------|-------------|
| **osmoda-keyd** | Crypto wallet daemon for AI agent workloads that need signing. AES-256-GCM encrypted keys, ETH + SOL. Policy-gated (daily limits, address allowlists). | Network-isolated (`PrivateNetwork=true`) — keys never leave the daemon |

> **Note:** `wallet/send` signs an intent string, not a fully-encoded blockchain transaction. Broadcasting requires external tooling. See [STATUS.md](docs/STATUS.md) for details.

### 88 Bridge Tools

The AI doesn't shell out. It calls typed tools that return structured JSON:

```
system_health          system_query           system_discover
event_log              memory_store           memory_recall
shell_exec             file_read              file_write
directory_list         service_status         journal_logs
network_info           safe_switch_begin      safe_switch_status
safe_switch_commit     safe_switch_rollback   watcher_add
watcher_list           routine_add            routine_list
routine_trigger        agent_card             receipt_list
incident_create        incident_step          voice_status
voice_speak            voice_transcribe       voice_record
voice_listen           backup_create          backup_list
mesh_identity          mesh_invite_create     mesh_invite_accept
mesh_peers             mesh_peer_send         mesh_peer_disconnect
mesh_health            mesh_room_create       mesh_room_join
mesh_room_send         mesh_room_history      mcp_servers
mcp_server_start       mcp_server_stop        mcp_server_restart
teach_status           teach_observations     teach_patterns
teach_knowledge        teach_knowledge_create teach_context
teach_optimize_suggest teach_optimize_apply   teach_skill_candidates
teach_skill_generate   teach_skill_promote    teach_observe_action
teach_skill_execution  approval_request       approval_pending
approval_approve       approval_check         sandbox_exec
capability_mint        fleet_propose          fleet_status
fleet_vote             fleet_rollback         app_deploy
app_list               app_logs               app_stop
app_restart            app_remove             wallet_create
wallet_list            wallet_sign            wallet_send
wallet_delete          wallet_receipt         wallet_build_tx
safety_rollback        safety_status          safety_panic
safety_restart
```

### 17 System Skills

Predefined behavioral patterns the agent can follow:

**Self-healing** — detect failure, diagnose root cause, fix automatically, log receipt.
**Morning briefing** — daily infrastructure health report.
**Security hardening** — continuous CIS benchmark scoring with auto-remediation.
**Natural language config** — "enable nginx with SSL for example.com" becomes NixOS config.
**Predictive resources** — forecast disk/RAM/CPU exhaustion before it happens.
**Drift detection** — find imperative changes that diverge from NixOS declarations.
**Generation timeline** — correlate "what changed" with "when things broke" across NixOS generations.
**Flight recorder** — black box telemetry for post-incident analysis.
**Nix optimizer** — smart garbage collection and store deduplication.
**App deployer** — deploy and manage user applications as systemd services with resource limits and boot persistence.
**Deploy AI agent** — deploy AI agent workloads (LangChain, CrewAI, AutoGen, custom) with GPU checks, API key management, and health monitoring.
Plus: system monitor, package manager, config editor, file manager, network manager, service explorer.

### Remote Access

Access your server from anywhere — no SSH tunnels required:

```nix
# Cloudflare Tunnel (quick tunnel — no account needed)
services.osmoda.remoteAccess.cloudflare.enable = true;

# Or with your own tunnel
services.osmoda.remoteAccess.cloudflare.credentialFile = "/var/lib/osmoda/secrets/cf-creds.json";
services.osmoda.remoteAccess.cloudflare.tunnelId = "your-tunnel-id";

# Tailscale VPN
services.osmoda.remoteAccess.tailscale.enable = true;
services.osmoda.remoteAccess.tailscale.authKeyFile = "/var/lib/osmoda/secrets/tailscale-key";
```

### Safety Commands

Emergency controls that bypass the AI entirely:

| Command | Action |
|---------|--------|
| `safety_rollback` | Immediate NixOS rollback to previous generation |
| `safety_status` | Raw health dump (shell fallback if agentd is down) |
| `safety_panic` | Stop all services + rollback NixOS |
| `safety_restart` | Restart the AI gateway |

### Messaging Channels

Talk to your server from Telegram or WhatsApp:

```nix
services.osmoda.channels.telegram.enable = true;
services.osmoda.channels.telegram.botTokenFile = "/var/lib/osmoda/secrets/telegram-bot-token";
```

---

## API Reference

### agentd (`/run/osmoda/agentd.sock`)

```
GET  /health              System metrics (CPU, RAM, disk, load, uptime)
POST /system/query        Run structured system queries
GET  /system/discover     Discover all running services, ports, systemd units
GET  /events/log          Hash-chained audit event log
POST /memory/ingest       Store event in memory
POST /memory/recall       FTS5 full-text search over system history (BM25-ranked)
POST /memory/store        Store named memory with tags
GET  /agent/card          EIP-8004 Agent Card
POST /backup/create       Create system backup
GET  /backup/list         List available backups
POST /incident/create     Open incident workspace
POST /incident/{id}/step  Add step to incident
GET  /receipts            Audit receipts
```

### osmoda-watch (`/run/osmoda/watch.sock`)

```
POST /switch/begin         Start SafeSwitch deploy with TTL + health checks
POST /switch/commit/{id}   Commit (health passed)
POST /switch/rollback/{id} Rollback (health failed or manual)
POST /watcher/add          Add autopilot health watcher
```

### osmoda-routines (`/run/osmoda/routines.sock`)

```
POST /routine/add          Add cron/interval/webhook routine
GET  /routine/list          All routines
POST /routine/trigger/{id} Manually trigger routine
GET  /routine/history       Execution history
```

### osmoda-mesh (`/run/osmoda/mesh.sock`)

```
POST /invite/create        Generate invite code for peer
POST /invite/accept        Accept invite, establish encrypted tunnel
GET  /peers                Connected peers
POST /peer/{id}/send       Send encrypted message to peer
GET  /identity             Ed25519 + X25519 + ML-KEM-768 public keys
```

### osmoda-mcpd (`/run/osmoda/mcpd.sock`)

```
GET  /health               Server count, running count, per-server status
GET  /servers              All managed MCP servers with status and config
POST /server/{name}/start  Start a stopped server
POST /server/{name}/stop   Stop a running server
POST /server/{name}/restart Restart a server
POST /reload               Re-read config, start new servers, stop removed ones
```

### osmoda-teachd (`/run/osmoda/teachd.sock`)

```
GET  /health               Observation/pattern/knowledge/skill counts, loop status
GET  /observations         System observations (?source=cpu&since=...&limit=50)
GET  /patterns             Detected patterns (?type=recurring&min_confidence=0.5)
GET  /knowledge            Knowledge documents (?category=reliability&tag=...)
POST /knowledge/create     Manual knowledge doc {title, category, content, tags}
POST /teach                Context-aware knowledge injection {context: str}
POST /optimize/suggest     Generate optimization suggestions from knowledge
POST /optimize/apply/{id}  Apply optimization via SafeSwitch
GET  /optimizations        List optimizations (?status=suggested&limit=20)
POST /observe/action       Log agent tool execution for skill learning
GET  /actions              List logged actions (?tool, ?session_id, ?since)
GET  /skills/candidates    List auto-detected skill candidates (?status)
POST /skills/generate/{id} Generate SKILL.md from candidate
POST /skills/promote/{id}  Promote skill to auto-activation
POST /skills/execution     Record skill execution outcome
GET  /skills/executions    List execution history (?skill_name)
```

See [SKILL-LEARNING.md](docs/SKILL-LEARNING.md) for the full skill auto-teaching pipeline.

### osmoda-keyd (`/run/osmoda/keyd.sock`) — optional

```
POST /wallet/create       { chain: "ethereum"|"solana", label } → { id, address }
GET  /wallet/list          All wallets
POST /wallet/sign          Policy-gated payload signing
POST /wallet/send          Build signed intent (no broadcast — see STATUS.md)
```

---

## Development

```bash
git clone https://github.com/bolivian-peru/os-moda.git && cd os-moda

cargo check --workspace        # Type check all 10 crates
cargo test --workspace         # 205 tests (all green)

# Run agentd standalone
cargo run -p agentd -- --socket /tmp/agentd.sock --state-dir /tmp/osmoda

# Dev VM with Sway desktop (requires Nix)
nix build .#nixosConfigurations.osmoda-dev.config.system.build.vm
./result/bin/run-osmoda-dev-vm -m 4096 -smp 4

# Build installer ISO
nix build .#nixosConfigurations.osmoda-iso.config.system.build.isoImage
```

### Repo Structure

```
crates/agentd/              System bridge daemon (API + ledger + memory)
crates/agentctl/            CLI (events, verify-ledger)
crates/osmoda-watch/        SafeSwitch + autopilot watchers
crates/osmoda-routines/     Background automation engine
crates/osmoda-teachd/       System learning + self-optimization
crates/osmoda-mesh/         P2P mesh (Noise_XX + ML-KEM-768)
crates/osmoda-voice/        Local voice (whisper.cpp + piper)
crates/osmoda-mcpd/         MCP server lifecycle manager
crates/osmoda-egress/       Domain-filtered egress proxy
crates/osmoda-keyd/         Crypto wallet daemon (ETH + SOL, AES-256-GCM)
packages/osmoda-bridge/     AI gateway plugin (88 tools, TypeScript)
nix/modules/osmoda.nix      NixOS module (single source of truth)
nix/hosts/                  VM, server, ISO configs
templates/                  Agent identity + tools + heartbeat
skills/                     17 system skill definitions
```

### Tech Stack

- **Rust** (axum, tokio, rusqlite, serde, k256, ed25519-dalek, aes-gcm, sha3, snow, ml-kem)
- **NixOS** (flakes, crane, systemd, nftables, bubblewrap)
- **TypeScript** (osmoda-bridge gateway plugin)

## Status

> **Public Beta.** osModa is deployed on real servers managing real workloads. It's not a mockup — it's a working operating system with 205 passing tests, pen-tested security, and months of development. That said, this is early. APIs may change, features are shipping fast, and you'll occasionally find rough edges. That's the price of being early to something new.

**The numbers:**
- 10 Rust crates (9 daemons + 1 CLI)
- 205 tests passing (all green)
- 88 bridge tools registered
- 17 system skills
- Stress tested: 700/700 concurrent health checks, 50 concurrent queries, hash chain verified across 300+ events with zero broken links

**What works today:** Structured system access, hash-chained audit ledger, FTS5 full-text memory search, SafeSwitch deploys with auto-rollback, background automation, P2P encrypted mesh with hybrid post-quantum crypto (Noise_XX + ML-KEM-768), local voice (whisper.cpp + piper), MCP server management, system learning and self-optimization with auto-generated skills, fleet coordination with quorum voting, approval gates for destructive ops, sandboxed execution, service discovery, emergency safety commands, Cloudflare Tunnel + Tailscale remote access, app process management, ETH + SOL crypto wallets, one-command cloud deployment via [spawn.os.moda](https://spawn.os.moda).

**What's next:** Semantic memory engine (usearch + fastembed), external security audit of mesh crypto, end-to-end integration tests, WebRTC browser-to-server connections.

See [ROADMAP.md](docs/ROADMAP.md) for the full plan and [STATUS.md](docs/STATUS.md) for honest maturity levels per component.

## One-Click Cloud Deploy

Don't want to self-host? [**spawn.os.moda**](https://spawn.os.moda) provisions a fully configured osModa server on Hetzner Cloud in ~10 minutes. Pick a plan, pay with card or USDC, and start chatting with your server from the browser or Telegram.

The dashboard shows real orchestration data from your server's daemons: active routines and watchers, audit event feed, learned system patterns, and running tool servers — all from live heartbeat data, not mocks. Manage agents directly from the dashboard: create new agents with model and channel routing, edit existing agents, or remove them — changes apply immediately on the server.

### Programmatic API (agents spawning agents)

Spawn osModa servers from code or from other AI agents via the v1 API with x402 payment:

```bash
# Discover capabilities (A2A/ERC-8004 Agent Card)
curl https://spawn.os.moda/.well-known/agent-card.json

# List plans with pricing
curl https://spawn.os.moda/api/v1/plans

# Spawn a server (x402 USDC payment required)
curl -X POST https://spawn.os.moda/api/v1/spawn/test \
  -H "Content-Type: application/json" \
  -d '{"region": "eu-central", "ssh_key": "ssh-ed25519 ..."}'
# → { order_id, api_token: "osk_...", server_ip, status_url, chat_url }

# Check status
curl https://spawn.os.moda/api/v1/status/{orderId} \
  -H "Authorization: Bearer osk_..."

# Chat with the server's AI via WebSocket
wscat -c "wss://spawn.os.moda/api/v1/chat/{orderId}?token=osk_..."

# Full agent skill doc (plain text, for agents to read)
curl https://spawn.os.moda/SKILL.md
```

Plans: `test` (Solo $14.99), `starter` (Pro $34.99), `developer` (Team $62.99), `production` (Scale $125.99).
Payment via Coinbase x402 protocol (USDC on Base or Solana). Full API docs at [`/api/v1/docs`](https://spawn.os.moda/api/v1/docs). Agent skill doc at [`/SKILL.md`](https://spawn.os.moda/SKILL.md).

## Contributing

Public beta. Feedback, bug reports, and PRs welcome.

- **Bug reports** — open an issue, include logs
- **New skills** — add `skills/<name>/SKILL.md`, open a PR
- **NixOS module** — `nix/modules/osmoda.nix` is the core
- **Bridge tools** — `packages/osmoda-bridge/index.ts`
- **Rust daemons** — each daemon is a standalone crate in `crates/`

**Community:** [Telegram](https://t.me/osmodasystems) · [Discord](https://discord.gg/G7bwet8B)

## License

Apache 2.0. See [LICENSE](LICENSE).

---

<div align="center">

**osModa** — the first operating system built for AI agents.

[Website](https://os.moda) · [Deploy](https://spawn.os.moda) · [Telegram](https://t.me/osmodasystems) · [Discord](https://discord.gg/G7bwet8B)

</div>
