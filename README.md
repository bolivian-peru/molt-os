<div align="center">

# osModa

**The AI doesn't manage your server. It *is* your server.**

A Rust + NixOS operating system where the AI agent runs at ring 0 with root access to every process, file, service, and kernel parameter. All mutations atomic and rollbackable. Every action logged to a tamper-proof hash-chained audit ledger. Third-party tools sandboxed. The agent is not.

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-8%20crates-orange.svg)](https://www.rust-lang.org/)
[![NixOS](https://img.shields.io/badge/NixOS-Atomic-5277C3.svg)](https://nixos.org/)
[![Tests](https://img.shields.io/badge/Tests-97%20passing-brightgreen.svg)]()
[![Tools](https://img.shields.io/badge/Agent%20Tools-45-blueviolet.svg)]()

[Quickstart](#quickstart) · [Architecture](#architecture) · [What It Does](#what-it-does) · [API](#api-reference) · [Development](#development)

[![Telegram](https://img.shields.io/badge/Telegram-Join-blue?logo=telegram)](https://t.me/osmodasystems)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.gg/G7bwet8B)

</div>

---

## Why This Exists

Traditional server management: SSH in, run commands, hope nothing breaks, write runbooks nobody reads, get paged at 3am.

osModa: the AI has structured access to the entire system through 45 typed tools exposed via 7 cooperating Rust daemons. It doesn't shell out and parse text — it calls `system_health`, gets structured JSON, makes decisions, and logs every action to a hash-chained ledger. If it breaks something, NixOS rolls back the entire system state atomically. If a service dies at 3am, `osmoda-watch` detects it, the agent diagnoses root cause, and SafeSwitch deploys a fix with automatic rollback if health checks fail.

The key insight: **NixOS makes AI root access safe.** Every system change is a transaction — it either fully applies or doesn't. Every state has a generation number. Rolling back is one command. The AI can be aggressive about fixing problems because the blast radius is bounded and reversible.

## Quickstart

### NixOS (flake) — 3 lines

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

### Any Linux Server — 1 command

```bash
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash
```

Converts Ubuntu/Debian to NixOS, builds 8 Rust binaries from source, installs the AI gateway + 45 tools, starts everything. Takes ~10 minutes.

**Supported:** Ubuntu 22.04+, Debian 12+, existing NixOS. x86_64 and aarch64.

### Deploy to Hetzner/DigitalOcean/AWS

```bash
git clone https://github.com/bolivian-peru/os-moda.git && cd os-moda
./scripts/deploy-hetzner.sh <server-ip> [ssh-key-path]
```

Or from the server directly:

```bash
# First run (installs NixOS via nixos-infect — server reboots)
curl -fsSL .../install.sh | sudo bash
# Second run (after reboot)
curl -fsSL .../install.sh | sudo bash -s -- --skip-nixos --api-key sk-ant-...
```

### Verify

```bash
# System health (structured JSON, not text parsing)
curl -s --unix-socket /run/osmoda/agentd.sock http://localhost/health | jq

# Audit ledger integrity
agentctl verify-ledger --state-dir /var/lib/osmoda

# Wallet daemon health
curl -s --unix-socket /run/osmoda/keyd.sock http://localhost/health | jq

# Mesh identity (post-quantum keys)
curl -s --unix-socket /run/osmoda/mesh.sock http://localhost/identity | jq
```

---

## Architecture

7 daemons, all Rust, communicating over Unix sockets. No daemon exposes TCP to the internet (except mesh peer port 18800, encrypted). The AI reaches the system exclusively through structured tool calls, never raw shell.

```
┌──────────────────────────────────────────────────────────────┐
│  User — Terminal / Web / Telegram / WhatsApp                  │
├──────────────────────────────────────────────────────────────┤
│  AI Gateway (OpenClaw)          reasoning + planning          │
│  osmoda-bridge                  45 typed tools                │
├────────┬──────────┬──────────┬──────────┬──────────┬────────┤
│ agentd │ keyd     │ watch    │ routines │ mesh     │ voice  │
│ System │ Crypto   │ Safe     │ Cron +   │ P2P      │ Local  │
│ bridge │ wallets  │ Switch   │ event    │ Noise_XX │ STT/   │
│ ledger │ ETH+SOL  │ rollback │ automate │ +ML-KEM  │ TTS    │
│ memory │ isolated │ watchers │          │ hybrid   │        │
├────────┴──────────┴──────────┴──────────┴──────────┴────────┤
│  osmoda-egress — domain-filtered CONNECT proxy (sandboxed)   │
├──────────────────────────────────────────────────────────────┤
│  NixOS — atomic rebuilds · instant rollback · generations    │
└──────────────────────────────────────────────────────────────┘
```

### Trust Model (3 rings)

```
RING 0  OpenClaw + agentd       Root. Full system. This is the agent.
RING 1  Approved apps           Sandboxed. Declared capabilities only.
RING 2  Untrusted tools         Max isolation. No network. Minimal filesystem.
```

The agent is ring 0 by design. It's not a chatbot with sudo — it's a system service with structured access to everything, constrained by NixOS atomicity and its own audit ledger, not by permission denials.

### Audit Ledger

Every mutation creates a hash-chained event in SQLite:

```
hash = SHA-256(id | ts | type | actor | payload | prev_hash)
```

Append-only. Tamper-evident. Any single modification invalidates the chain. Verifiable offline with `agentctl verify-ledger`.

---

## What It Does

### Daemon Breakdown

| Daemon | What it does | Socket | Key feature |
|--------|-------------|--------|-------------|
| **agentd** | System bridge: processes, services, network, filesystem, NixOS config, kernel params. Hash-chained audit ledger. Vector memory (semantic search over system events). Agent Card (EIP-8004). Backups. | `/run/osmoda/agentd.sock` | The kernel-level bridge between AI and OS |
| **osmoda-keyd** | Crypto wallet daemon. AES-256-GCM encrypted keys. ETH + SOL signing. JSON policy engine (daily limits, address allowlists). Keys never leave the daemon. | `/run/osmoda/keyd.sock` | Runs with `PrivateNetwork=true` — zero network access |
| **osmoda-watch** | SafeSwitch: deploy with a timer, health checks, and automatic rollback if anything fails. Autopilot watchers: deterministic health checks with escalation (restart -> rollback -> notify). | `/run/osmoda/watch.sock` | Blue-green deploys with automatic undo |
| **osmoda-routines** | Background cron/event/webhook automation. Runs between conversations. Health checks, log scans, service monitors, scheduled tasks. | `/run/osmoda/routines.sock` | Agent actions that persist when nobody's chatting |
| **osmoda-mesh** | P2P encrypted agent-to-agent communication. Noise_XX (X25519/ChaChaPoly/BLAKE2s) + ML-KEM-768 hybrid post-quantum. Invite-based pairing. No central server. | `/run/osmoda/mesh.sock` + TCP 18800 | Agents talk to each other, end-to-end encrypted |
| **osmoda-voice** | Local speech-to-text (whisper.cpp) + text-to-speech (piper). All processing on-device. No cloud APIs. No data leaves the machine. | `/run/osmoda/voice.sock` | Fully local voice, zero cloud dependency |
| **osmoda-egress** | HTTP CONNECT proxy with domain allowlist per capability token. Only path to the internet for sandboxed tools. | `127.0.0.1:19999` | Sandboxed tools can't phone home |

### 45 Bridge Tools

The AI doesn't shell out. It calls typed tools that return structured JSON:

```
system_health          system_query           event_log
memory_store           memory_recall          memory_ingest
shell_exec             file_read              file_write
directory_list         service_status         journal_logs
network_info           wallet_create          wallet_list
wallet_sign            wallet_send            wallet_delete
wallet_receipt         safe_switch_begin      safe_switch_status
safe_switch_commit     safe_switch_rollback   watcher_add
watcher_list           routine_add            routine_list
routine_trigger        agent_card             receipt_list
incident_create        incident_step          voice_status
voice_speak            voice_transcribe       voice_record
voice_listen           backup_create          backup_list
mesh_identity          mesh_invite_create     mesh_invite_accept
mesh_peers             mesh_peer_send         mesh_peer_disconnect
mesh_health
```

### 15 System Skills

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
Plus: system monitor, package manager, config editor, file manager, network manager, service explorer.

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
GET  /events/log          Hash-chained audit event log
POST /memory/ingest       Store event in vector memory
POST /memory/recall       Semantic search over system history
POST /memory/store        Store named memory with tags
GET  /agent/card          EIP-8004 Agent Card
POST /backup/create       Create system backup
GET  /backup/list         List available backups
POST /incident/create     Open incident workspace
POST /incident/{id}/step  Add step to incident
GET  /receipts            Audit receipts
```

### osmoda-keyd (`/run/osmoda/keyd.sock`)

```
POST /wallet/create       { chain: "ethereum"|"solana", label } → { id, address }
GET  /wallet/list          All wallets
POST /wallet/sign          Policy-gated payload signing
POST /wallet/send          Build signed transaction (no broadcast)
```

### osmoda-watch (`/run/osmoda/watch.sock`)

```
POST /switch/begin         Start SafeSwitch deploy with TTL + health checks
POST /switch/commit/{id}   Commit (health passed)
POST /switch/rollback/{id} Rollback (health failed or manual)
POST /watcher/add          Add autopilot health watcher
```

### osmoda-mesh (`/run/osmoda/mesh.sock`)

```
POST /invite/create        Generate invite code for peer
POST /invite/accept        Accept invite, establish encrypted tunnel
GET  /peers                Connected peers
POST /peer/{id}/send       Send encrypted message to peer
GET  /identity             Ed25519 + X25519 + ML-KEM-768 public keys
```

### osmoda-routines (`/run/osmoda/routines.sock`)

```
POST /routine/add          Add cron/interval/webhook routine
GET  /routine/list          All routines
POST /routine/trigger/{id} Manually trigger routine
GET  /routine/history       Execution history
```

---

## Development

```bash
git clone https://github.com/bolivian-peru/os-moda.git && cd os-moda

cargo check --workspace        # Type check all 8 crates
cargo test --workspace         # 97 tests

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
crates/osmoda-keyd/         Crypto wallet daemon (ETH + SOL, AES-256-GCM)
crates/osmoda-watch/        SafeSwitch + autopilot watchers
crates/osmoda-routines/     Background automation engine
crates/osmoda-egress/       Domain-filtered egress proxy
crates/osmoda-voice/        Local voice (whisper.cpp + piper)
crates/osmoda-mesh/         P2P mesh (Noise_XX + ML-KEM-768)
packages/osmoda-bridge/     AI gateway plugin (45 tools, TypeScript)
nix/modules/osmoda.nix      NixOS module (single source of truth)
nix/hosts/                  VM, server, ISO configs
templates/                  Agent identity + tools + heartbeat
skills/                     15 system skill definitions
```

### Tech Stack

- **Rust** (axum, tokio, rusqlite, serde, k256, ed25519-dalek, aes-gcm, sha3, snow, ml-kem)
- **NixOS** (flakes, crane, systemd, nftables, bubblewrap)
- **TypeScript** (osmoda-bridge gateway plugin)

## Status

Early beta. 8 Rust crates, 97 tests passing, 45 bridge tools, 7 daemons, 15 system skills.

**Proven on hardware:** Full deployment tested on Hetzner Cloud (CX22). All daemons start, all sockets respond, wallet creation works, mesh identity generates, audit ledger chains correctly.

**What works:** Structured system access, hash-chained audit ledger, ETH + SOL crypto signing, SafeSwitch deploys with auto-rollback, background automation, P2P encrypted mesh with hybrid post-quantum crypto, local voice, all 45 bridge tools.

**What's next:** Vector memory engine, web dashboard, `POST /nix/rebuild` API, multi-model support (Claude/Grok/Llama), MCP protocol, fleet coordination, encrypted filesystem, Tor hidden service access.

See [ROADMAP.md](docs/ROADMAP.md) for the full plan.

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

**osModa** — your server fixes itself at 3am. You sleep.

</div>
