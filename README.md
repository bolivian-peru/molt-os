<div align="center">

<img src="assets/server-brain-os-moda.png" alt="osModa — AI-native operating system" width="100%"/>

# osModa

**Your server fixes itself at 3am. You sleep.**

An AI-native operating system built on NixOS. The agent isn't running *on* your server — it *is* your server. Root access. Every process. Every file. Every config. All changes atomic, rollbackable, and logged to a tamper-proof audit ledger.

[![License: Apache 2.0](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-orange.svg)](https://www.rust-lang.org/)
[![NixOS](https://img.shields.io/badge/NixOS-Powered-5277C3.svg)](https://nixos.org/)
[![Tests](https://img.shields.io/badge/Tests-71%20passing-brightgreen.svg)]()
[![Status](https://img.shields.io/badge/Status-Early%20Beta-orange.svg)]()

[Quickstart](#quickstart) · [Why NixOS?](#why-nixos) · [Architecture](#architecture) · [Components](#components) · [Contributing](#contributing)

[![Telegram](https://img.shields.io/badge/Telegram-Join-blue?logo=telegram)](https://t.me/osmodasystems)
[![Discord](https://img.shields.io/badge/Discord-Join-5865F2?logo=discord&logoColor=white)](https://discord.gg/G7bwet8B)

</div>

---

## Quickstart

### NixOS (flake)

Already running NixOS? Three lines:

```nix
# flake.nix — add osModa as an input
inputs.os-moda.url = "github:bolivian-peru/os-moda";

# configuration.nix
imports = [ os-moda.nixosModules.default ];
services.osmoda.enable = true;
```

```bash
sudo nixos-rebuild switch
# Open http://localhost:18789 — talk to your server
```

### One command (any Linux server)

```bash
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash
```

This will:
1. Convert your server to NixOS (Ubuntu/Debian — asks before proceeding)
2. Build all osModa daemons from source
3. Install OpenClaw AI gateway + 37 system tools
4. Start everything — open `http://localhost:18789` to chat with your OS

**Supported:** Ubuntu 22.04+, Debian 12+, existing NixOS. x86_64 and aarch64.

### Existing NixOS (without flake)

```bash
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash -s -- --skip-nixos
```

### With API key (skip setup wizard)

```bash
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash -s -- --api-key sk-ant-...
```

### Hetzner / DigitalOcean / AWS

```bash
# SSH into your fresh VPS, then:
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash
# After reboot (NixOS install), SSH back in and re-run:
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash -s -- --skip-nixos
```

Or deploy from your local machine:

```bash
git clone https://github.com/bolivian-peru/os-moda.git && cd osmoda
./scripts/deploy-hetzner.sh <server-ip> [ssh-key-path]
```

### After install

```bash
# Chat with your OS (web UI)
ssh -L 18789:localhost:18789 root@<server-ip>
open http://localhost:18789

# Health check
curl -s --unix-socket /run/osmoda/agentd.sock http://localhost/health | jq

# Verify audit ledger integrity
agentctl verify-ledger --state-dir /var/lib/osmoda

# View event log
agentctl events --state-dir /var/lib/osmoda --limit 20
```

---

## What is osModa?

osModa turns a bare machine into an AI-managed system. The AI doesn't SSH in from outside — it runs as a first-class OS service with structured access to every process, file, service, and kernel parameter through `agentd`, a Rust daemon exposing the entire system over a Unix socket API.

Every action the AI takes is recorded in an append-only, hash-chained audit ledger. Every system change goes through NixOS, making it atomic, reproducible, and rollbackable.

```
3:17 AM   nginx goes down
3:17 AM   osmoda-watch detects failure via health watcher
3:17 AM   Diagnosis: config corruption, last good in generation 47
3:18 AM   NixOS rollback to generation 47 — nginx restored
3:18 AM   Receipt logged to hash-chained audit ledger
3:18 AM   47 seconds total downtime, zero human involvement
```

## Why NixOS?

Giving an AI root access to a mutable Linux system is terrifying. NixOS makes it safe:

- **Atomic rebuilds** — every config change is a transaction. It works or it doesn't. No half-applied states.
- **Instant rollback** — if the AI breaks something, `nixos-rebuild switch --rollback` restores the last working state in seconds. osModa's SafeSwitch does this automatically.
- **Reproducible** — the entire system state is defined in `.nix` files. You can rebuild the exact same system on another machine from the config alone.
- **Generations** — NixOS keeps a history of every system state. The AI can correlate "what changed" with "when things broke" by walking the generation timeline.

Without NixOS, AI-driven system changes are a one-way door. With NixOS, every door has an undo button.

## Architecture

osModa runs as a set of cooperating daemons, each communicating over Unix sockets. No TCP. No HTTP to the internet. Everything stays local.

```
┌─────────────────────────────────────────────────────────┐
│  User (Terminal / Web Chat / API)                        │
├─────────────────────────────────────────────────────────┤
│  OpenClaw Gateway              AI reasoning + tools      │
│  osmoda-bridge plugin          37 registered tools       │
├────────────┬────────────┬────────────┬──────────────────┤
│  agentd    │ osmoda-keyd│osmoda-watch│osmoda-routines   │
│  System    │ Crypto     │ SafeSwitch │ Background       │
│  bridge    │ wallets    │ + watchers │ automation       │
│  + ledger  │ (isolated) │            │                  │
├────────────┴────────────┴────────────┴──────────────────┤
│  NixOS                                                   │
│  Atomic rebuilds · Instant rollback · Reproducible       │
└─────────────────────────────────────────────────────────┘
```

### Trust Rings

```
RING 0: OpenClaw + agentd          Full system, root-equivalent
RING 1: Approved apps              Sandboxed, declared capabilities
RING 2: Untrusted tools            Max isolation, no network, minimal fs
```

### Audit Ledger

Every mutation creates a hash-chained event in SQLite:

```
hash = SHA-256(id|ts|type|actor|payload|prev_hash)
```

Tamper-proof. Verifiable with `agentctl verify-ledger`.

## Components

| Daemon | Role | Socket |
|--------|------|--------|
| **agentd** | System bridge, audit ledger, memory, Agent Card, backups | `/run/osmoda/agentd.sock` |
| **osmoda-keyd** | ETH + SOL wallets, AES-256-GCM, policy engine. Zero network access | `/run/osmoda/keyd.sock` |
| **osmoda-watch** | SafeSwitch deploys with auto-rollback, health watchers | `/run/osmoda/watch.sock` |
| **osmoda-routines** | Cron/interval automation (health checks, log scans) | `/run/osmoda/routines.sock` |
| **osmoda-egress** | Domain-filtered HTTP CONNECT proxy for sandboxed tools | `127.0.0.1:19999` |
| **osmoda-voice** | Local STT (whisper.cpp) + TTS (piper) — no cloud APIs | `/run/osmoda/voice.sock` |
| **osmoda-bridge** | OpenClaw plugin — 37 tools wiring all daemons to AI | TypeScript |

### 15 System Skills

Self-healing, morning briefing, security hardening, natural language NixOS config, predictive resource alerts, drift detection, generation timeline debugging, flight recorder, Nix store optimizer, system monitor, package manager, config editor, file manager, network manager, service explorer.

## Development

```bash
git clone https://github.com/bolivian-peru/os-moda.git && cd osmoda

cargo check --workspace
cargo test --workspace    # 71 tests

# Run agentd locally
cargo run -p agentd -- --socket /tmp/agentd.sock --state-dir /tmp/osmoda

# Dev VM (requires Nix with flakes)
nix build .#nixosConfigurations.osmoda-dev.config.system.build.vm
./result/bin/run-osmoda-dev-vm -m 4096 -smp 4

# Build installer ISO
nix build .#nixosConfigurations.osmoda-iso.config.system.build.isoImage
```

## Project Status

Early beta. 7 Rust daemons, 71 tests passing, 37 bridge tools, 15 system skills. Production-hardened with subprocess timeouts, graceful shutdown, input validation, daily backups, and systemd security directives.

**Working and tested:**
- All 7 Rust daemons compile and pass tests
- ETH + SOL crypto signing with known-vector verification
- Hash-chained audit ledger with tamper detection
- SafeSwitch state machine with auto-rollback
- Cron scheduler, routine persistence, background automation
- OpenClaw plugin loads and registers all 37 tools

**Needs real-world testing:**
- Full NixOS VM boot-to-chat pipeline
- End-to-end daemon communication under load
- NixOS rollback via SafeSwitch on a live system

## Roadmap

What's shipping next.

**In progress:**
- Vector memory engine (USearch backend) — semantic log search, incident correlation, real agent memory
- Web dashboard — full browser-based OS management with built-in terminal
- `POST /nix/rebuild` API — agent-triggered NixOS rebuilds through an audited endpoint
- Persistent SafeSwitch sessions — survive daemon restarts during probation

**Coming soon:**
- Multi-model support — swap between Claude, Grok, Llama, or any local model as the agentic backend. One config change, not a rewrite
- Encrypted filesystem — LUKS full-disk encryption baked into the NixOS module. Everything at rest is encrypted by default
- Tor hidden service — access your osModa instance from anywhere via `.onion` address. No port forwarding, no public IP exposure
- Web shell relay — local command line access through the browser. Manage your server from a phone if you have to
- MCP protocol support — expose agentd via Model Context Protocol so any MCP-compatible agent can manage the system
- Prometheus metrics — `GET /metrics` endpoint for plugging into existing monitoring stacks
- Configurable autonomy — three modes: `suggest` (explain), `supervised` (ask first), `autonomous` (act + log receipt)
- Fleet coordination — manage multiple osModa machines from a single pane
- Hedgehog mode — friendly assistant personality for when you want your server to feel less like a server

See [discussions](https://github.com/bolivian-peru/os-moda/discussions) for what's being worked on and what's next.

## Contributing

We're in early beta and actively looking for feedback. Every issue, bug report, and idea helps.

- **Bug reports** — open an issue, include logs if possible
- **Feature ideas** — open an issue, describe the use case
- **New skills** — add a `skills/<name>/SKILL.md` and open a PR
- **NixOS module improvements** — `nix/modules/osmoda.nix` is the core
- **Bridge tools** — add tools in `packages/osmoda-bridge/index.ts`

For larger changes, open an issue first so we can discuss the approach before you invest time.

**Community:** [Telegram](https://t.me/osmodasystems) · [Discord](https://discord.gg/G7bwet8B)

## Tech Stack

- **Rust**: axum, rusqlite, tokio, serde, k256, ed25519-dalek, aes-gcm, sha3, clap
- **TypeScript**: OpenClaw plugin (osmoda-bridge)
- **Nix**: flakes, crane (Rust builds), flake-utils
- **NixOS**: systemd services, nftables, bubblewrap

## Repo Structure

```
crates/agentd/              Kernel bridge daemon (system API + ledger)
crates/agentctl/            CLI tool (events, verify-ledger)
crates/osmoda-keyd/         Crypto wallet daemon (ETH + SOL)
crates/osmoda-watch/        SafeSwitch + autopilot watchers
crates/osmoda-routines/     Background automation engine
crates/osmoda-egress/       Egress proxy
crates/osmoda-voice/        Local voice (STT + TTS)
packages/osmoda-bridge/     OpenClaw plugin (37 tools)
nix/modules/osmoda.nix      NixOS module
nix/hosts/                  VM, server, ISO configs
templates/                  Agent identity, tools, heartbeat
skills/                     15 system skills
```

## License

Apache 2.0. See [LICENSE](LICENSE).

---

<div align="center">

**osModa** — your server fixes itself at 3am. You sleep.

Built with NixOS, Rust, and OpenClaw.

</div>
