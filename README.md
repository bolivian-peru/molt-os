<div align="center">

# 🛡️ Thorox

**Your server has a brain now.**

Self-healing infrastructure powered by NixOS + AI.
It watches. It learns. It fixes. You sleep.

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/Built%20with-Rust-orange.svg)](https://www.rust-lang.org/)
[![NixOS](https://img.shields.io/badge/NixOS-Powered-5277C3.svg)](https://nixos.org/)
[![OpenClaw](https://img.shields.io/badge/OpenClaw-Compatible-red.svg)](https://openclaw.ai/)

[Install](#install) · [30-Second Demo](#demo) · [Why Thorox?](#why-thorox) · [How It Works](#how-it-works) · [Features](#features) · [Docs](docs/)

</div>

---

> *"I stopped SSHing into my servers. Now I just text them on Telegram."*

---

## What Happens at 3 AM

```
[3:17 AM]  nginx goes down. Config file corrupted.
[3:17 AM]  Thorox detects failure via heartbeat monitor.
[3:17 AM]  Thorox diagnoses: missing nginx config, last working in generation 47.
[3:18 AM]  Thorox runs NixOS rollback → generation 47 restored.
[3:18 AM]  nginx is back. 47 seconds total downtime.
[3:18 AM]  Thorox messages you on Telegram: "Fixed it. Here's what happened."
[3:18 AM]  Audit entry #52 recorded in hash-chained ledger.

You wake up at 8 AM. Everything's fine.
Your morning briefing explains what happened.
```

## Install

One command. 60 seconds. Works on any Linux server.

```bash
curl -fsSL https://thorox.dev/install | bash
```

Or with an API key:

```bash
curl -fsSL https://thorox.dev/install | bash -s -- --api-key sk-ant-...
```

Then open `http://localhost:18789` and start talking to your server.

**Supported:** Ubuntu 22.04+, Debian 12+, NixOS
**Tested on:** Hetzner Cloud, DigitalOcean, bare metal

## Demo

<!-- Replace with actual asciinema embed or GIF -->
<!-- [![Demo](https://asciinema.org/a/XXXXX.svg)](https://asciinema.org/a/XXXXX) -->

```
You:     "What's eating my RAM?"
Thorox:  PostgreSQL is using 4.2 GB — 3 idle connections
         holding 800 MB each. Want me to add PgBouncer?

You:     "Yes"
Thorox:  Here's the NixOS config I'll add:
           services.pgbouncer.enable = true;
           services.pgbouncer.databases = { ... };
         This creates generation 48. Rollbackable. Apply?

You:     "Apply"
Thorox:  Done. RAM: 5.8 GB → 2.1 GB.
         Audit entry #53 recorded.
```

**Self-healing demo:** Break nginx → Thorox detects → diagnoses root cause → NixOS rollback → fixed. Under 60 seconds. [Watch video →](docs/)

**Morning briefing:** Every day at 7 AM, a Telegram message with service health, resource trends, security events, and overnight incidents.

## Why Thorox?

Everyone's building AI agents that run **ON** a server.
Thorox is the first server that **IS** an AI agent.

| | Without Thorox | With Thorox |
|---|---|---|
| 3 AM crash | PagerDuty wakes you | Thorox fixes it, tells you at breakfast |
| Bad deploy | Hope you have backups | NixOS rollback — instant, atomic |
| "What changed?" | `grep bash_history` | Hash-chained audit trail with reasoning |
| New service | Write Ansible playbooks | "Set up PostgreSQL with nightly backups" |
| Security | Quarterly scan, maybe | Continuous score + auto-hardening |
| Config drift | Invisible, permanent | Detected and reconciled |
| Reproduce server | Pray your docs are right | NixOS config IS the server |

### vs. Other OpenClaw Setups

| | Clawezy / VivaClaw / etc. | Nathan's Self-Healing Blog | Thorox |
|---|---|---|---|
| System awareness | ❌ Shell commands only | ⚠️ SSH-based | ✅ Native daemon (agentd) |
| Atomic rollback | ❌ Ubuntu/Debian | ❌ Ansible/Terraform | ✅ NixOS generations |
| Audit trail | ❌ None | ⚠️ Git history | ✅ Hash-chained, tamper-proof |
| Self-healing | ❌ | ⚠️ Manual scripts | ✅ Automatic + rollback |
| OS-level memory | ❌ | ❌ | ✅ Persistent context |
| Continuous monitoring | ❌ | ⚠️ Cron | ✅ Always-on watchers |

## How It Works

Thorox = **NixOS** (the body) + **agentd** (the nervous system) + **OpenClaw** (the brain)

```
┌─────────────────────────────────────────────────────┐
│  You (Telegram / Web Chat / SSH / API)              │
├─────────────────────────────────────────────────────┤
│  OpenClaw AI Gateway        Claude / Sonnet         │
│  Agent reasoning · Skills · Memory                  │
├─────────────────────────────────────────────────────┤
│  agentd (Rust daemon)       The Nervous System      │
│  ├─ system_health           CPU, RAM, disk, uptime  │
│  ├─ service_status          systemd services        │
│  ├─ journal_logs            Log analysis            │
│  ├─ shell_exec              Controlled root access  │
│  ├─ file_read / file_write  Filesystem access       │
│  ├─ network_info            Ports, interfaces       │
│  ├─ event_log               Audit ledger queries    │
│  └─ memory_store / recall   Long-term memory        │
├─────────────────────────────────────────────────────┤
│  NixOS                      The Body                │
│  ├─ Atomic rebuilds         All-or-nothing deploys  │
│  ├─ Instant rollback        Any generation, 1 cmd   │
│  └─ Reproducible            Config = server         │
└─────────────────────────────────────────────────────┘
```

The AI doesn't run shell commands and hope for the best.
It proposes **declarative state transitions** on a formally-specified system.
Every change is reviewable, reversible, and auditable.

## Features

🔧 **Self-Healing Infrastructure**
Monitors services, detects failures, diagnoses root causes, auto-remediates via NixOS atomic rollback. Every action logged to the hash-chained audit ledger.

💬 **Natural Language DevOps**
"Install Caddy as reverse proxy for port 3000" → generates NixOS config → shows diff → applies atomically. No more learning Nix language.

⏰ **Morning Briefing**
Daily Telegram report: service health, resource trends, security events, overnight incidents, cost tracking.

🕐 **Time-Travel Debugging**
"What broke overnight?" → correlates NixOS generations + audit ledger + journal logs → pinpoints the exact config change that caused the failure.

🔒 **Security Autopilot**
Continuous posture scoring. Auto-fixes safe issues (fail2ban, SSH hardening, exposed ports). Presents a score with actionable recommendations.

📊 **Predictive Resource Management**
Trend analysis on disk, memory, CPU usage. Alerts days before exhaustion. Proposes and applies NixOS config fixes.

🧹 **Configuration Drift Detection**
Finds manual changes outside NixOS management — imperative packages, ad-hoc cron jobs, hand-edited configs. Offers to bring everything into declarative management.

✈️ **Flight Recorder**
Black box telemetry for your server. Continuous state snapshots for post-incident forensics.

🧠 **Intelligent Nix Optimization**
Smart garbage collection that knows which generations to keep (current, last-known-good, backup baseline) and which to clean.

## Built on OpenClaw

Thorox extends the [OpenClaw](https://openclaw.ai) ecosystem. It's not a fork — it's the **infrastructure layer** that gives OpenClaw agents system-level superpowers.

If OpenClaw is the brain, Thorox is the nervous system.

- **agentd** provides native OS awareness (not just shell commands)
- **Hash-chained ledger** provides tamper-proof audit trail
- **NixOS** provides atomic, rollbackable system state
- **Skills** teach the agent OS-level workflows

## Architecture

### Trust Rings

```
RING 0: OpenClaw + agentd     Full system access, root-equivalent
RING 1: Approved apps          Sandboxed, declared capabilities
RING 2: Untrusted tools        Max isolation, no network, minimal fs
```

### Event Ledger

Every system action is recorded in SQLite with SHA-256 hash chaining:

```sql
-- Every event references the previous hash.
-- Break the chain → instant detection.
hash = SHA-256(id || ts || type || actor || payload || prev_hash)
```

Tamper-proof. SOC2-auditor friendly. Every AI action is accountable.

## Development

```bash
nix develop                  # Enter dev shell (Rust, Node.js, Nix tools)
cargo check --workspace      # Verify compilation
cargo test --workspace       # Run tests
cargo run -p agentd          # Run agentd locally

# Build the dev VM
nix build .#nixosConfigurations.agentos-dev.config.system.build.vm

# Build the installer ISO
nix build .#nixosConfigurations.agentos-iso.config.system.build.isoImage
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for full development guide.

## Roadmap

- [x] agentd system daemon with hash-chained ledger
- [x] OpenClaw bridge plugin (12 system tools)
- [x] NixOS module for declarative deployment
- [x] Self-healing, security hardening, morning briefing skills
- [x] Hetzner Cloud deployment
- [ ] One-command curl installer
- [ ] Telegram channel integration
- [ ] ZVEC vector memory (semantic search)
- [ ] Voice interface (whisper.cpp + piper-tts)
- [ ] Fleet mode (multi-server shared intelligence)
- [ ] Home Assistant bridge
- [ ] Desktop kiosk mode

## License

MIT. See [LICENSE](LICENSE).

---

<div align="center">

**Thorox** — your server has a brain now.

Built with NixOS, Rust, and OpenClaw.

[⭐ Star this repo](../../stargazers) · [🐛 Report bug](../../issues) · [💡 Request feature](../../issues)

</div>
