<div align="center">

# AgentOS

**The first server that IS an AI agent — not just running one.**

Self-healing infrastructure. Natural language DevOps. Hash-chained audit trail.
Built on NixOS for atomic, rollbackable system management.

[Install](#install) · [Demo](#demo) · [How It Works](#how-it-works) · [Features](#features) · [Architecture](#architecture)

</div>

---

## What is this?

Everyone's building AI agents that run ON a server. AgentOS is the first server that IS an AI agent.

```
You:    "Install PostgreSQL with nightly backups"
Thorox: Here's the NixOS config I'd add:
          services.postgresql.enable = true;
          services.postgresqlBackup = { ... };
        This creates generation 48. Rollbackable. Apply?

You:    "Something broke after the last update"
Thorox: Generation 48 updated OpenSSL 3.1→3.2.
        Your app uses a deprecated TLS cipher.
        Rolling back to generation 47... Done. App is back.
        Audit entry #52 recorded.

[3 AM, you're asleep]
Thorox: nginx went down. Config file was corrupted.
        Rolled back to last known-good generation.
        nginx is running. Sending you the incident report.
```

You stop SSHing into servers. You just talk to them.

---

## Install

One command:

```bash
curl -fsSL https://raw.githubusercontent.com/moltOS/molt-os/main/scripts/install.sh | sudo bash
```

Or with an API key:

```bash
curl -fsSL ... | sudo bash -s -- --api-key sk-ant-...
```

Then open `http://localhost:18789` and start talking to your server.

**Supported platforms:** Ubuntu 22.04+, Debian 12+, existing NixOS
**Tested on:** Hetzner Cloud, DigitalOcean, bare metal

---

## Demo

**Self-healing:** Break nginx → AgentOS detects it → diagnoses root cause → NixOS rollback → fixed. 60 seconds.

**Natural language DevOps:** "Set up Caddy as a reverse proxy for my app on port 3000" → generates NixOS config → shows diff → applies atomically.

**Morning briefing:** Every day at 7am you get a server health report with security events, resource trends, and overnight incidents.

<!-- TODO: demo video link -->

---

## How It Works

AgentOS = **NixOS** + **agentd** + **OpenClaw** + **hash-chained audit ledger**

```
┌──────────────────────────────────────────────────────┐
│  You (chat / Telegram / API)                         │
├──────────────────────────────────────────────────────┤
│  OpenClaw AI Gateway          port 18789             │
│  Claude Opus / Sonnet         agent reasoning        │
├──────────────────────────────────────────────────────┤
│  agentos-bridge plugin        12 system tools        │
│  ├─ system_health             CPU, RAM, disk, uptime │
│  ├─ shell_exec                full root access       │
│  ├─ file_read / file_write    filesystem access      │
│  ├─ service_status            systemd services       │
│  ├─ journal_logs              log analysis           │
│  ├─ network_info              ports, interfaces      │
│  ├─ event_log                 audit ledger queries   │
│  └─ memory_store / recall     long-term memory       │
├──────────────────────────────────────────────────────┤
│  agentd                       Rust daemon on Unix    │
│  ├─ System queries            socket (/run/agentos/) │
│  ├─ Hash-chained ledger       tamper-proof audit     │
│  └─ Memory system             persistent context     │
├──────────────────────────────────────────────────────┤
│  NixOS                        declarative OS layer   │
│  ├─ Atomic rebuilds           nixos-rebuild switch   │
│  ├─ Rollback                  instant generation     │
│  │                            revert                 │
│  └─ Reproducible              config IS the server   │
└──────────────────────────────────────────────────────┘
```

The AI agent doesn't just run shell commands. It proposes **declarative state transitions** on a formally-specified system. Every change is reviewable, reversible, and auditable.

---

## Features

### Self-Healing Infrastructure
AgentOS monitors services, detects failures, diagnoses root causes, and auto-remediates using NixOS atomic rollback. Every action logged to the hash-chained audit ledger.

### Natural Language System Configuration
Describe what you want in plain English. AgentOS generates the NixOS config, shows you the diff, and applies it atomically. No more learning Nix language to get NixOS benefits.

### Generation-Aware Time-Travel Debugging
"What broke and when?" AgentOS correlates NixOS generations + audit ledger + journal logs to pinpoint exactly which config change caused which failure.

### Predictive Resource Management
Trend analysis on disk, memory, and CPU usage. Alerts you days before exhaustion. Proposes and applies NixOS config fixes (log rotation, swap, garbage collection).

### Security Hardening Score
Continuous security posture assessment. Auto-fixes safe issues (fail2ban, SSH hardening, port closing). Presents a score with actionable recommendations.

### Configuration Drift Detection
Detects manual changes that exist outside NixOS management — imperative packages, ad-hoc cron jobs, manually edited configs. Offers to bring everything into declarative management.

### Flight Recorder
Black box telemetry for your server. Continuous state snapshots for post-incident forensics. "What happened at 3 AM?" answered with timestamped evidence.

### Intelligent Nix Store Optimization
Smart garbage collection that knows which generations to keep (current, last-known-good, backup baselines) and which to clean. Reclaims disk with surgical precision.

### Morning Briefing
Daily infrastructure report: service health, resource trends, security events, overnight incidents, and cost tracking. Screenshot-ready for your team Slack.

---

## Why NixOS?

The combination of AI + NixOS enables features that are **genuinely impossible** on Ubuntu/Debian/RHEL:

| Feature | Ubuntu + AI | AgentOS (NixOS + AI) |
|---------|-------------|---------------------|
| Rollback a bad change | Hope you have backups | `nixos-rebuild switch --rollback` — instant |
| Reproduce server | Pray your Ansible is accurate | Config file IS the server — identical rebuild |
| Audit trail | grep through bash history | Hash-chained tamper-proof ledger |
| Config drift | Invisible and permanent | Detected and reconcilable |
| Atomic deploys | apt-get halfway through and crash? | All-or-nothing generation switch |

---

## Architecture

### Trust Rings

```
RING 0: OpenClaw + agentd     Full system access, root-equivalent
RING 1: Approved apps          Sandboxed, declared capabilities
RING 2: Untrusted tools        Max isolation, no network, minimal fs
```

### Components

- **agentd** — Rust daemon on Unix socket. System queries, hash-chained event ledger, memory endpoints.
- **agentos-bridge** — OpenClaw plugin. 12 tools that give the AI structured system access.
- **agentos-egress** — Localhost HTTP proxy with domain allowlist for sandboxed tools.
- **Skills** — Markdown instructions that teach the agent OS-level workflows (self-healing, security hardening, etc.)

### Event Ledger

Every system action is recorded in a SQLite database with SHA-256 hash chaining:

```sql
CREATE TABLE events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts TEXT NOT NULL,
  type TEXT NOT NULL,
  actor TEXT NOT NULL,
  payload TEXT NOT NULL,
  prev_hash TEXT NOT NULL,
  hash TEXT NOT NULL  -- SHA-256(id || ts || type || actor || payload || prev_hash)
);
```

Tamper-proof. Every event references the previous hash. Break the chain → instant detection.

---

## Repository Layout

```
flake.nix                       NixOS flake (OS configurations + Rust builds)
Cargo.toml                      Rust workspace
crates/
  agentd/                       System daemon (axum + rusqlite + tokio)
  agentctl/                     CLI tool (verify-ledger, events)
  agentos-egress/               Egress proxy (domain allowlist)
  agentos-voice/                Voice pipeline (whisper.cpp + piper-tts)
packages/
  agentos-bridge/               OpenClaw plugin (12 system tools)
nix/
  modules/agentos.nix           Core NixOS module
  modules/agentos-shell.nix     Kiosk desktop shell
  modules/agentos-setup.nix     First-boot setup wizard
  hosts/dev-vm.nix              QEMU development VM
  hosts/server.nix              Headless server
  hosts/hetzner.nix             Hetzner Cloud deployment
  hosts/iso.nix                 Installer ISO
skills/                         Agent skill definitions
  self-healing/                 Detect + diagnose + auto-fix
  morning-briefing/             Daily infrastructure report
  security-hardening/           Continuous security scoring
  natural-language-config/      NixOS config from plain English
  drift-detection/              Find imperative drift
  generation-timeline/          Time-travel debugging
  predictive-resources/         Resource exhaustion prediction
  nix-optimizer/                Smart Nix store management
  flight-recorder/              Server black box telemetry
  system-monitor/               Real-time system monitoring
  ...
templates/                      Agent identity + personality
scripts/
  install.sh                    One-command installer
  deploy-hetzner.sh             Hetzner deployment automation
```

---

## Development

```bash
# Enter dev shell (provides Rust, Node.js, Nix tools)
nix develop

# Check everything compiles
cargo check --workspace

# Run tests
cargo test --workspace

# Run agentd locally
cargo run -p agentd -- --socket /tmp/agentd.sock --state-dir /tmp/agentos

# Build the dev VM
nix build .#nixosConfigurations.agentos-dev.config.system.build.vm
./result/bin/run-agentos-dev-vm -m 4096 -smp 4

# Build the ISO
nix build .#nixosConfigurations.agentos-iso.config.system.build.isoImage
```

---

## Deploy to Hetzner Cloud

```bash
# Generate SSH key
ssh-keygen -t ed25519 -f .keys/agentos_hetzner -N ""

# Create a CX22 server with Ubuntu 24.04 in Hetzner dashboard
# Add your SSH public key in Hetzner > Security > SSH Keys

# Deploy
./scripts/deploy-hetzner.sh <server-ip> .keys/agentos_hetzner

# Access (via SSH tunnel)
ssh -i .keys/agentos_hetzner -L 18789:localhost:18789 root@<server-ip>
# Open http://localhost:18789 in your browser
```

---

## Roadmap

- [x] agentd system daemon with hash-chained ledger
- [x] OpenClaw bridge plugin (12 system tools)
- [x] NixOS module for declarative deployment
- [x] First-boot setup wizard
- [x] Self-healing, security hardening, morning briefing skills
- [x] One-command installer
- [x] Hetzner Cloud deployment
- [ ] Telegram channel integration
- [ ] ZVEC vector memory (semantic search, auto-injection)
- [ ] Desktop kiosk mode (Sway + full-screen chat)
- [ ] Voice interface (whisper.cpp + piper-tts)
- [ ] Fleet mode (multi-server shared intelligence)
- [ ] One-command curl installer from GitHub

---

## License

MIT. See [LICENSE](LICENSE).

---

<div align="center">

**AgentOS** — the server that thinks.

Built with NixOS, Rust, and OpenClaw.

</div>
