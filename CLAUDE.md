# CLAUDE.md — molt-os (AgentOS)

## What this is

AgentOS (molt-os): NixOS distribution where OpenClaw IS the operating system.
Not an agent running on an OS. The agent IS the OS.

OpenClaw has FULL system access. Root. All files. All processes. All APIs.
The sandbox exists for UNTRUSTED third-party tools, not for OpenClaw itself.

## Architecture (3 trust rings)

```
RING 0: OpenClaw + agentd (full system, root-equivalent)
  ↓ grants capabilities to ↓
RING 1: Approved apps (sandboxed, declared capabilities)
  ↓ even more restricted ↓
RING 2: Untrusted tools (max isolation, no network, minimal fs)
```

## Components

1. **agentd** (Rust) — kernel bridge daemon. Unix socket API at `/run/agentos/agentd.sock`.
   Gives OpenClaw structured access to: processes, services, network, filesystem, NixOS config,
   kernel params, users, firewall. Append-only hash-chained event log in SQLite.
   Memory system endpoints for ingest/recall/store.

2. **agentos-bridge** (TypeScript) — OpenClaw plugin. Registers tools via
   `api.registerTool()` factory pattern (12 tools): system_health, system_query,
   event_log, memory_store, memory_recall, shell_exec, file_read, file_write,
   directory_list, service_status, journal_logs, network_info.
   M1+ files (memory-backend.ts, memory-engine.ts, voice-client.ts) are present but not yet wired.

3. **agentos-egress** (Rust) — localhost-only HTTP CONNECT proxy. Domain allowlist
   per capability token. Only path to internet for sandboxed tools.

4. **System Skills** (SKILL.md) — self-healing, morning-briefing, security-hardening,
   natural-language-config, predictive-resources, drift-detection, generation-timeline,
   flight-recorder, nix-optimizer, system-monitor, system-packages, system-config,
   file-manager, network-manager, service-explorer.

5. **NixOS module** (agentos.nix) — single module that wires everything as systemd services.

## Repo layout

```
./CLAUDE.md                              # This file (canonical project doc)
./flake.nix                              # Root flake (NixOS + Rust via crane)
./Cargo.toml                             # Rust workspace root
./nix/modules/agentos.nix                # NixOS module (THE core config file)
./nix/hosts/dev-vm.nix                   # QEMU dev VM (Sway desktop)
./nix/hosts/server.nix                   # Headless server config
./nix/hosts/iso.nix                      # Installer ISO config
./crates/agentd/                         # Rust: kernel bridge daemon
  ├── Cargo.toml
  └── src/
      ├── main.rs                        # Entry + socket setup
      ├── api/                           # HTTP handlers
      │   ├── mod.rs
      │   ├── health.rs                  # GET /health
      │   ├── system.rs                  # POST /system/query
      │   ├── events.rs                  # GET /events/log
      │   └── memory.rs                  # /memory/ingest, /memory/recall, /memory/store
      ├── ledger.rs                      # SQLite event log + hash chain
      └── state.rs                       # Shared app state
./crates/agentctl/                       # Rust: CLI (events, verify-ledger)
  ├── Cargo.toml
  └── src/main.rs
./crates/agentos-egress/                 # Rust: egress proxy
  ├── Cargo.toml
  └── src/main.rs
./packages/agentos-bridge/               # TypeScript: OpenClaw plugin
  ├── package.json                       # OpenClaw plugin format (openclaw.extensions)
  ├── openclaw.plugin.json               # Plugin manifest (id + kind)
  ├── index.ts                           # Plugin entry — 12 tools via api.registerTool()
  ├── agentd-client.ts                   # HTTP-over-Unix-socket client for agentd
  ├── memory-engine.ts                   # ZVEC collection management (M1+)
  ├── memory-backend.ts                  # OpenClaw memory backend (M1+)
  └── voice-client.ts                    # Voice daemon client (M1+)
./packages/agentos-system-skills/        # Skill collection package
./skills/
  ├── self-healing/SKILL.md              # Detect + diagnose + auto-fix failures
  ├── morning-briefing/SKILL.md          # Daily infrastructure health report
  ├── security-hardening/SKILL.md        # Continuous security scoring + auto-fix
  ├── natural-language-config/SKILL.md   # NixOS config from plain English
  ├── predictive-resources/SKILL.md      # Resource exhaustion prediction
  ├── drift-detection/SKILL.md           # Find imperative config drift
  ├── generation-timeline/SKILL.md       # Time-travel debugging via generations
  ├── flight-recorder/SKILL.md           # Server black box telemetry
  ├── nix-optimizer/SKILL.md             # Smart Nix store management
  ├── system-monitor/SKILL.md            # Real-time system monitoring
  ├── system-packages/SKILL.md
  ├── system-config/SKILL.md
  ├── file-manager/SKILL.md
  ├── network-manager/SKILL.md
  └── service-explorer/SKILL.md
./templates/
  ├── AGENTS.md                          # "You ARE the operating system"
  ├── SOUL.md                            # Calm, competent, omniscient
  ├── TOOLS.md                           # All agentd endpoints documented
  ├── IDENTITY.md                        # Agent identity (name, role, trust model)
  ├── USER.md                            # Learned user preferences template
  └── HEARTBEAT.md                       # Periodic task scheduling template
./scripts/
  ├── install.sh                         # One-command installer (curl | bash)
  └── deploy-hetzner.sh                  # Push deploy from local to Hetzner
./docs/
  ├── ARCHITECTURE.md                    # Architecture overview
  ├── DEMO-SCRIPT.md                     # Product Hunt demo recording script
  ├── AGENTOS-PRODUCT-HUNT-STRATEGY.md   # Launch strategy
  └── planning/                          # Archived planning docs
      ├── MASTER-PLAN.md
      ├── MEMORY-SYSTEM.md
      └── MEMORY-LLM-INTEGRATION.md
```

## Tech stack

- **Rust**: agentd, agentctl, egress proxy (axum, rusqlite, tokio, sha2, clap)
- **TypeScript**: agentos-bridge OpenClaw plugin
- **Nix**: flakes, crane (Rust builds), flake-utils (multi-system), nixos-generators
- **NixOS**: systemd services, nftables, bubblewrap
- **Desktop**: Sway (Wayland), kitty, Firefox
- **Memory**: ZVEC (in-process vector DB), nomic-embed-text-v2-moe (768-dim), SQLite FTS5

## Build + run

```bash
# Dev VM (the primary feedback loop)
nix build .#nixosConfigurations.agentos-dev.config.system.build.vm
./result/bin/run-agentos-dev-vm -m 4096 -smp 4

# ISO
nix build .#nixosConfigurations.agentos-iso.config.system.build.isoImage

# Validate
nix flake check

# Rust
cargo check --workspace
cargo test --workspace

# agentd standalone (development)
cargo run -p agentd -- --socket /tmp/agentd.sock --state-dir /tmp/agentos
```

## Implementation order

1. **flake.nix** — all inputs, nixosConfigurations for vm + iso
2. **crates/agentd** — minimal: /health + /system/query(processes) + /events/log + hash chain
3. **nix/modules/agentos.nix** — agentd + gateway as systemd services
4. **nix/hosts/dev-vm.nix** — Sway desktop, auto-login, gateway running
5. **packages/agentos-bridge** — register system_query + system_health as OpenClaw tools
6. **skills/system-monitor/SKILL.md** — first skill
7. **templates/** — agent identity
8. **BUILD VM AND TEST END-TO-END** — don't proceed until this works
9. Expand agentd: /system/mutate, /nix/rebuild, /nix/search
10. More skills, sandbox, egress proxy, agentctl

## agentd API reference

```
GET  /health              → { cpu, ram, disk, load, uptime }
POST /system/query        { query: str, args: obj } → JSON result
GET  /events/log          ?type=...&actor=...&limit=N → events[]
POST /memory/ingest       { event: MemoryEvent } → { id }
POST /memory/recall       { query: str, max_results: num, timeframe: str } → chunks[]
POST /memory/store        { summary: str, detail: str, category: str, tags: str[] } → { id }
GET  /memory/health       → { model_status, collection_size }
```

Future (M1+):
```
POST /system/mutate       { mutation: str, args: obj, reason: str } → result + event_id
POST /nix/rebuild         { changes: str, dry_run: bool } → result + generation
POST /nix/search          { query: str } → packages[]
POST /sandbox/exec        { command: str, capabilities: str[], timeout_sec: num } → output
POST /capability/mint     { granted_to: str, permissions: str[], ttl_sec: num } → token
```

## Event log schema (SQLite)

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
-- hash = SHA-256(id || ts || type || actor || payload || prev_hash)
```

## Memory system (M0 — simplified ZVEC)

### Engine
- Single ZVEC collection (NOT three tiers — defer tiering to M2)
- Single embedding model: nomic-embed-text-v2-moe (Q8_0, 512MB, 768-dim)
- RRF (Reciprocal Rank Fusion) for hybrid merge — simpler than weighted scores
- SQLite FTS5 alongside ZVEC for BM25 keyword search
- Relevance score threshold for injection count (NOT hardcoded 6)
- Token budget cap: max ~1500 tokens injected per prompt
- Query embedding cache: LRU with 5-minute TTL
- Contextual enrichment at ingestion (prepend context to chunks before embedding)

### Binding strategy (M0)
Node.js `@zvec/zvec` in the agentos-bridge plugin. agentd handles ledger + watchers,
the plugin handles vector search. No Rust FFI complexity.

### Ground truth
Markdown files remain source of truth (OpenClaw compatible).
ZVEC indexes are derived — always rebuildable from files.
Path: `/var/lib/agentos/memory/`

### Deferred to M2+
- Second tier (Hot + Archive split)
- Technical embedding model (nomic-embed-code)
- System watchers (process, service, journal)
- Watcher event summarization
- Pattern detection and user model
- Proactive alerts

## Coding rules

- Rust: axum + rusqlite + tokio + serde. Error handling: anyhow. Tests: `#[cfg(test)]`.
- Nix: mkOption/mkIf/mkEnableOption. Flakes only. crane for Rust builds.
- TypeScript: OpenClaw plugin conventions. `api.registerAgentTool()` for tools.
- Skills: YAML frontmatter + markdown. Reference agentd tools. No secrets.
- All system mutations go through agentd (never raw shell from OpenClaw).
- Every mutation creates a hash-chained event.
- `nix flake check` must pass at all times.
- `cargo check --workspace` must pass at all times.

## Non-negotiables

1. agentd runs as root (it IS the system)
2. Every mutation logged with hash chain
3. Destructive ops require user approval
4. Third-party tools sandboxed (bubblewrap + egress proxy)
5. NixOS config = source of truth (not imperative changes)
6. VM must boot and work end-to-end before adding more features
7. ISO buildable from flake
8. Memory: markdown files are ground truth, ZVEC is derived index
