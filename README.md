<div align="center">

# osModa

**The AI-native operating system.**

A NixOS distribution where the AI agent *is* the operating system.
Not an app running on Linux. The agent has root. It sees everything. It fixes everything.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-orange.svg)](https://www.rust-lang.org/)
[![NixOS](https://img.shields.io/badge/NixOS-Powered-5277C3.svg)](https://nixos.org/)

[Architecture](#architecture) · [Components](#components) · [Build](#build--run) · [Status](#project-status) · [Docs](docs/)

</div>

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

### agentd (Rust)
The kernel bridge daemon. Runs as root. Exposes the entire system over a Unix socket API (`/run/osmoda/agentd.sock`). Provides: system health, structured queries, hash-chained event log, memory system (ingest/recall/store), EIP-8004 Agent Card identity, structured receipts, and incident workspaces.

### osmoda-keyd (Rust)
Crypto wallet daemon for ETH and SOL. Runs with `PrivateNetwork=true` — zero network access. Private keys encrypted with AES-256-GCM. Policy engine enforces daily spend limits and signing caps. Proper Keccak-256 for Ethereum address derivation, ed25519-dalek for Solana. Key material zeroized on drop.

### osmoda-watch (Rust)
SafeSwitch deploy transactions and autopilot health watchers. Start a change with a TTL and health checks — if checks fail, auto-rollback to the previous NixOS generation. Watchers run deterministic health checks on interval with escalation: restart service, rollback generation, notify.

### osmoda-routines (Rust)
Background cron/interval/event automation engine. Runs scheduled tasks between conversations: health checks every 5 minutes, service monitoring every 10 minutes, log scans every 15 minutes. Cron expression parser, persistent routine definitions.

### osmoda-bridge (TypeScript)
OpenClaw plugin that wires all daemons to the AI. 37 tools registered via `api.registerTool()`: system management, wallets, SafeSwitch, watchers, routines, identity, receipts, incidents.

### osmoda-egress (Rust)
Localhost-only HTTP CONNECT proxy with domain allowlist. The only path to the internet for sandboxed Ring 2 tools.

### NixOS Module (osmoda.nix)
Single module that wires everything as systemd services. `services.osmoda.enable = true` activates the full stack with proper systemd hardening (PrivateNetwork, RestrictAddressFamilies, NoNewPrivileges).

## Build + Run

```bash
# Dev VM (primary feedback loop)
nix build .#nixosConfigurations.osmoda-dev.config.system.build.vm
./result/bin/run-osmoda-dev-vm -m 4096 -smp 4

# Installer ISO
nix build .#nixosConfigurations.osmoda-iso.config.system.build.isoImage

# Validate
nix flake check

# Rust workspace
cargo check --workspace
cargo test --workspace

# Run agentd standalone (development)
cargo run -p agentd -- --socket /tmp/agentd.sock --state-dir /tmp/osmoda
```

### Requirements

- Nix with flakes enabled
- Rust 1.85+ (for local development)
- x86_64-linux target (NixOS builds are Linux-only)

## Project Status

osModa is under active development. See [docs/STATUS.md](docs/STATUS.md) for an honest assessment of what works and what's still placeholder.

**What compiles and has tests:**
- All 6 Rust crates compile clean (`cargo check --workspace`)
- 71 unit tests pass (`cargo test --workspace`)
- Crypto signing: ETH (k256 ECDSA + Keccak-256) and SOL (ed25519-dalek) with sign/verify roundtrip tests
- AES-256-GCM key encryption with known-vector tests
- Cron expression parser with edge case tests
- SafeSwitch state machine with health check execution
- Hash-chained audit ledger with tamper detection

**What needs real-world testing:**
- Full NixOS VM boot-to-chat pipeline
- Daemon-to-daemon communication over Unix sockets
- OpenClaw plugin integration with all 37 tools
- NixOS rollback via SafeSwitch in a live system

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
packages/osmoda-bridge/     OpenClaw plugin (37 tools)
nix/modules/osmoda.nix      NixOS module
nix/hosts/                  VM, server, ISO configs
templates/                  Agent identity, tools, heartbeat
skills/                     Self-healing, security, monitoring skills
docs/                       Architecture, status, planning
```

## License

MIT. See [LICENSE](LICENSE).

---

<div align="center">

**osModa** — the AI-native operating system.

Built with NixOS, Rust, and OpenClaw.

</div>
