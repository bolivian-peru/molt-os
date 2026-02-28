<div align="center">

# osModa

**Your server has an AI brain. It monitors, fixes, deploys, and explains — without you SSH-ing in.**

A NixOS distribution with AI-native system management. 9 Rust daemons give the AI structured access to your entire server — processes, services, network, config, deploys. Every action is logged to a tamper-proof audit ledger. Every system change is atomic and rollbackable.

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-10%20crates-orange.svg)](https://www.rust-lang.org/)
[![NixOS](https://img.shields.io/badge/NixOS-Atomic-5277C3.svg)](https://nixos.org/)
[![Tests](https://img.shields.io/badge/Tests-136%20passing-brightgreen.svg)]()
[![Tools](https://img.shields.io/badge/Agent%20Tools-72-blueviolet.svg)]()

[Quickstart](#quickstart) · [First 5 Minutes](#what-happens-in-the-first-5-minutes) · [Architecture](#architecture) · [Safety](#safety-model) · [API](#api-reference) · [Development](#development)

[![Telegram](https://img.shields.io/badge/Telegram-Join-blue?logo=telegram)](https://t.me/osmodasystems)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.gg/G7bwet8B)

</div>

---

## Who Is This For

- **Solo developers who run their own servers** — your server monitors itself, fixes problems at 3am, and tells you about it in the morning
- **AI agent builders who need a managed runtime** — deploy agents with GPU access, API key management, health monitoring, and crash recovery built in
- **Small teams tired of on-call rotations** — the AI handles routine ops, escalates what it can't fix, and keeps a complete audit trail

## Why This Exists

Servers break at 3am. Nobody's awake. The fix is usually "SSH in, check logs, restart the service" — but by the time you do that, users have already noticed.

Current AI agent tooling makes this worse: shell out, parse text, hope the regex holds, no audit trail, no rollback, manual recovery when things go sideways.

osModa gives the AI structured access to the entire OS through 72 typed tools exposed via 9 Rust daemons. No shell parsing. `system_health` returns structured JSON. Every mutation is hash-chained to a tamper-proof ledger. If a deploy breaks something, NixOS rolls back the entire system state atomically. If a service dies at 3am, the watcher detects it, the agent diagnoses root cause, SafeSwitch deploys a fix — with automatic rollback if health checks fail.

**Why NixOS?** Every system change is a transaction. Every state has a generation number. Rolling back is one command. The blast radius of any configuration change is bounded and reversible. This makes AI root access meaningfully safer than on a traditional Linux distribution. (NixOS rollback covers OS state — not data sent to external APIs or deleted user data. See [Safety Model](#safety-model).)

## What Happens in the First 5 Minutes

1. **Install** — add the flake to your NixOS config and `nixos-rebuild switch`
2. **Open the web chat** — the AI greets you and runs a health check on your server
3. **Ask "How's my server doing?"** — the AI calls `system_health` and `system_discover`, shows you what's running
4. **Say "Set up Telegram"** — it walks you through creating a bot and connecting it, so you can message your server from your phone
5. **Try something real** — "Install nginx and set up a reverse proxy for port 3000" — the AI edits NixOS config, rebuilds via SafeSwitch with auto-rollback if anything breaks

See the full [Getting Started Guide](docs/GETTING-STARTED.md) for a detailed walkthrough with expected output at each step.

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

Converts Ubuntu/Debian to NixOS, builds 10 Rust binaries from source, installs the AI gateway + 72 tools, starts everything. Takes ~10 minutes.

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
│  ├─ osmoda agent (Opus)         72 tools · 17 skills · full access · web      │
│  └─ mobile agent (Sonnet)       full access · concise replies · Telegram/WA    │
│  osmoda-bridge                  72 typed tools (shared plugin)                 │
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

### Trust Model (3 rings)

```
RING 0  OpenClaw + agentd       Root. Full system. This is the agent.
RING 1  Approved apps           Sandboxed. Declared capabilities only.
RING 2  Untrusted tools         Max isolation. No network. Minimal filesystem.
```

The agent is ring 0 by design. It's not a chatbot with sudo — it's a system service with structured access to everything, constrained by NixOS atomicity and its own audit ledger, not by permission denials. Lower rings cannot escalate privileges upward by design. Ring 0 remains the trusted computing base and must be governed by approval policies, spending limits, and audit review.

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
| **Safety commands** | `safety_rollback`, `safety_panic`, `safety_status`, `safety_restart` bypass the AI entirely — the user always has an escape hatch. |
| **Pentest verified** | Full automated pentest: injection attacks (SQL, path traversal, shell), payload bombs, error hardening, stress testing (700/700 concurrent health checks). All pass. |

### What NixOS rollback covers — and what it doesn't

**Covered:** OS configuration, package state, service definitions, firewall rules, system generations. Any bad config change can be atomically reverted.

**NOT covered:** Data already sent to external APIs, signed crypto transactions, deleted user data, exposed secrets, or side effects on remote systems. Ring 0 access means the agent can do anything the OS can do — the safety model relies on structured tools, audit trails, and NixOS atomicity, not on restricting the agent's access.

### What's planned but not yet implemented

- **Approval gate for destructive ops** — currently convention-based (agent prompt says "ask before destructive actions") but not enforced by code. This is the #1 priority for the next release.
- **Ring 1/Ring 2 sandbox** — the trust ring model is designed but not yet enforced. Third-party tools currently run without bubblewrap isolation.
- **Capability tokens** — fine-grained, time-limited access tokens for socket authentication. Currently file-permissions only.
- **External security audit** — mesh crypto uses standard primitives (Noise_XX, ML-KEM-768) but hasn't had independent review.

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
| **agentd** | System bridge: processes, services, network, filesystem, NixOS config, kernel params. Hash-chained audit ledger. FTS5 memory search. | The structured interface between AI and OS |
| **osmoda-watch** | SafeSwitch: deploy with a timer, health checks, and automatic rollback if anything fails. Autopilot watchers with escalation (restart -> rollback -> notify). | Blue-green deploys with automatic undo |
| **osmoda-routines** | Background cron/event/webhook automation. Runs between conversations. Health checks, log scans, service monitors. | Agent actions that persist when nobody's chatting |
| **osmoda-teachd** | OBSERVE loop (30s) collects metrics. LEARN loop (5m) detects patterns. TEACH API injects knowledge. Optimizer suggests fixes. | The OS learns from its own behavior |

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

### 72 Bridge Tools

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
teach_optimize_suggest teach_optimize_apply
app_deploy             app_list               app_logs
app_stop               app_restart            app_remove
wallet_create          wallet_list            wallet_sign
wallet_send            wallet_delete          wallet_receipt
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
GET  /health               Observation/pattern/knowledge counts, loop status
GET  /observations         System observations (?source=cpu&since=...&limit=50)
GET  /patterns             Detected patterns (?type=recurring&min_confidence=0.5)
GET  /knowledge            Knowledge documents (?category=reliability&tag=...)
POST /knowledge/create     Manual knowledge doc {title, category, content, tags}
POST /teach                Context-aware knowledge injection {context: str}
POST /optimize/suggest     Generate optimization suggestions from knowledge
POST /optimize/apply/{id}  Apply optimization via SafeSwitch
GET  /optimizations        List optimizations (?status=suggested&limit=20)
```

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
cargo test --workspace         # 136 tests

# Run agentd locally
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
packages/osmoda-bridge/     AI gateway plugin (72 tools, TypeScript)
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

> **Early beta.** This is a working prototype, not production-grade infrastructure. Use on disposable servers or development environments. Expect rough edges.

10 Rust crates (9 daemons + 1 CLI), 136 tests passing, 72 bridge tools, 17 system skills.

**Tested on hardware:** Full deployment tested on Hetzner Cloud (CX22/CX23). All 9 daemons start, all sockets respond, wallet creation works, mesh identity generates, audit ledger chains correctly, teachd observes and learns. Stress tested: 100 concurrent health checks per daemon (700/700 OK), 50 concurrent complex queries, 20 rapid wallet create/delete cycles, hash chain verified across 300+ events with zero broken links.

**What works now:** Structured system access, hash-chained audit ledger, FTS5 full-text memory search, SafeSwitch deploys with auto-rollback, background automation, P2P encrypted mesh with hybrid post-quantum crypto, local voice, MCP server management, system learning and self-optimization, service discovery, emergency safety commands, Cloudflare Tunnel + Tailscale remote access, app process management with systemd-run, ETH + SOL crypto signing, all 72 bridge tools.

**What's next:** Approval gate for destructive ops, web dashboard with live chat, semantic memory engine (usearch + fastembed), Ring 1/Ring 2 sandbox implementation, external security audit of mesh crypto.

See [ROADMAP.md](docs/ROADMAP.md) for the full plan and [STATUS.md](docs/STATUS.md) for honest maturity levels per component.

## Contributing

Early beta. Feedback welcome.

- **Bug reports** — open an issue, include logs
- **New skills** — add `skills/<name>/SKILL.md`, open a PR
- **NixOS module** — `nix/modules/osmoda.nix` is the core
- **Bridge tools** — `packages/osmoda-bridge/index.ts`

**Community:** [Telegram](https://t.me/osmodasystems) · [Discord](https://discord.gg/G7bwet8B)

## License

Apache 2.0. See [LICENSE](LICENSE).

---

<div align="center">

**osModa** — your server fixes itself at 3am.

</div>
