# CLAUDE.md — osModa

## What this is

osModa: NixOS distribution where OpenClaw IS the operating system.
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

1. **agentd** (Rust) — kernel bridge daemon. Unix socket API at `/run/osmoda/agentd.sock`.
   Gives OpenClaw structured access to: processes, services, network, filesystem, NixOS config,
   kernel params, users, firewall. Append-only hash-chained event log in SQLite.
   Memory system endpoints for ingest/recall/store.
   Agent Card (EIP-8004) identity + capability discovery.
   Structured receipts + incident workspaces for auditable troubleshooting.

2. **osmoda-bridge** (TypeScript) — OpenClaw plugin. Registers tools via
   `api.registerTool()` factory pattern (45 tools): system_health, system_query,
   event_log, memory_store, memory_recall, shell_exec, file_read, file_write,
   directory_list, service_status, journal_logs, network_info,
   wallet_create, wallet_list, wallet_sign, wallet_send, wallet_delete, wallet_receipt,
   safe_switch_begin, safe_switch_status, safe_switch_commit, safe_switch_rollback,
   watcher_add, watcher_list, routine_add, routine_list, routine_trigger,
   agent_card, receipt_list, incident_create, incident_step,
   voice_status, voice_speak, voice_transcribe, voice_record, voice_listen,
   backup_create, backup_list,
   mesh_identity, mesh_invite_create, mesh_invite_accept, mesh_peers,
   mesh_peer_send, mesh_peer_disconnect, mesh_health.

3. **osmoda-egress** (Rust) — localhost-only HTTP CONNECT proxy. Domain allowlist
   per capability token. Only path to internet for sandboxed tools.

4. **osmoda-keyd** (Rust) — OS-native crypto wallet daemon. Unix socket at `/run/osmoda/keyd.sock`.
   AES-256-GCM encrypted keys, policy-gated signing (daily limits), ETH + SOL wallets.
   Runs with PrivateNetwork=true (zero network access). Keys never leave keyd.
   SignerBackend trait allows future MPC/HSM/Vault integration.

5. **osmoda-watch** (Rust) — SafeSwitch + autopilot watchers. Unix socket at `/run/osmoda/watch.sock`.
   Deploy transactions with timer + health gates + auto-rollback.
   Autopilot watchers: deterministic health checks with escalation (restart → rollback → notify).

6. **osmoda-routines** (Rust) — background cron/event/webhook automation engine.
   Unix socket at `/run/osmoda/routines.sock`.
   Runs scheduled tasks between agent conversations (health checks, service monitors, log scans).
   Default routines match HEARTBEAT.md cadences.

7. **osmoda-mesh** (Rust) — P2P encrypted agent-to-agent communication daemon. Unix socket at `/run/osmoda/mesh.sock`,
   TCP listener at port 18800. Noise_XX (X25519/ChaChaPoly/BLAKE2s) + ML-KEM-768 hybrid post-quantum.
   Invite-based pairing, no central server. Ed25519 identity signatures.

8. **System Skills** (SKILL.md) — self-healing, morning-briefing, security-hardening,
   natural-language-config, predictive-resources, drift-detection, generation-timeline,
   flight-recorder, nix-optimizer, system-monitor, system-packages, system-config,
   file-manager, network-manager, service-explorer.

9. **NixOS module** (osmoda.nix) — single module that wires everything as systemd services.
   Generates OpenClaw config file from NixOS options (channels, auth, plugins).
   Channel options: `channels.telegram` and `channels.whatsapp` — config generation
   and credential management; actual connections handled by OpenClaw.

## Repo layout

```
./CLAUDE.md                              # This file (canonical project doc)
./flake.nix                              # Root flake (NixOS + Rust via crane)
./Cargo.toml                             # Rust workspace root
./nix/modules/osmoda.nix                 # NixOS module (THE core config file)
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
      │   ├── memory.rs                  # /memory/ingest, /memory/recall, /memory/store
      │   ├── agent_card.rs              # GET /agent/card, POST /agent/card/generate
      │   └── receipts.rs               # GET /receipts, /incident/* endpoints
      ├── ledger.rs                      # SQLite event log + hash chain
      └── state.rs                       # Shared app state
./crates/agentctl/                       # Rust: CLI (events, verify-ledger)
  ├── Cargo.toml
  └── src/main.rs
./crates/osmoda-egress/                  # Rust: egress proxy
  ├── Cargo.toml
  └── src/main.rs
./crates/osmoda-keyd/                    # Rust: crypto wallet daemon
  ├── Cargo.toml
  └── src/
      ├── main.rs                        # Entry + socket setup
      ├── signer.rs                      # SignerBackend trait + LocalKeyBackend (ETH + SOL)
      ├── policy.rs                      # JSON policy engine (daily limits, allowlists)
      ├── receipt.rs                     # Receipt logging to agentd ledger
      └── api.rs                         # Axum handlers (/wallet/*)
./crates/osmoda-watch/                   # Rust: SafeSwitch + autopilot watchers
  ├── Cargo.toml
  └── src/
      ├── main.rs                        # Entry + background loops
      ├── switch.rs                      # SafeSwitch state machine + health checks
      ├── watcher.rs                     # Autopilot watcher definitions + execution
      └── api.rs                         # Axum handlers (/switch/*, /watcher/*)
./crates/osmoda-routines/                # Rust: background automation engine
  ├── Cargo.toml
  └── src/
      ├── main.rs                        # Entry + scheduler loop
      ├── routine.rs                     # Routine definitions + action execution
      ├── scheduler.rs                   # Cron parser + interval scheduler
      └── api.rs                         # Axum handlers (/routine/*)
./crates/osmoda-mesh/                    # Rust: P2P encrypted agent-to-agent mesh
  ├── Cargo.toml
  └── src/
      ├── main.rs                        # Entry + TCP listener + background tasks
      ├── identity.rs                    # Ed25519 + X25519 + ML-KEM-768 keypairs
      ├── handshake.rs                   # Noise_XX + hybrid PQ key exchange
      ├── transport.rs                   # Encrypted TCP connection lifecycle
      ├── messages.rs                    # MeshMessage enum + wire framing
      ├── invite.rs                      # Out-of-band invite codes (base64url)
      ├── peers.rs                       # Peer storage + connection state
      ├── api.rs                         # Axum handlers (/invite/*, /peer/*, /identity, /health)
      └── receipt.rs                     # Audit logging to agentd ledger
./packages/osmoda-bridge/                # TypeScript: OpenClaw plugin
  ├── package.json                       # OpenClaw plugin format (openclaw.extensions)
  ├── openclaw.plugin.json               # Plugin manifest (id + kind)
  ├── index.ts                           # Plugin entry — 45 tools via api.registerTool()
  ├── keyd-client.ts                     # HTTP-over-Unix-socket client for keyd
  ├── watch-client.ts                    # HTTP-over-Unix-socket client for watch
  ├── routines-client.ts                 # HTTP-over-Unix-socket client for routines
  ├── voice-client.ts                    # Voice daemon client
  └── mesh-client.ts                     # HTTP-over-Unix-socket client for mesh
./packages/osmoda-system-skills/         # Skill collection package
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
  ├── ARCHITECTURE.md                    # Architecture overview (all 7 daemons)
  ├── STATUS.md                          # Honest maturity assessment per component
  ├── DEMO-SCRIPT.md                     # Demo recording script
  ├── GO-TO-MARKET.md                    # Launch strategy
  └── planning/                          # Archived planning docs
      ├── MASTER-PLAN.md
      ├── MEMORY-SYSTEM.md
      ├── MEMORY-LLM-INTEGRATION.md
      └── TRENDABILITY-ANALYSIS.md
```

## Tech stack

- **Rust**: agentd, agentctl, egress proxy, keyd, watch, routines, mesh (axum, rusqlite, tokio, sha2, clap, k256, ed25519-dalek, aes-gcm, sha3, snow, ml-kem)
- **TypeScript**: osmoda-bridge OpenClaw plugin
- **Nix**: flakes, crane (Rust builds), flake-utils (multi-system), nixos-generators
- **NixOS**: systemd services, nftables, bubblewrap
- **Desktop**: Sway (Wayland), kitty, Firefox
- **Memory**: ZVEC (in-process vector DB), nomic-embed-text-v2-moe (768-dim), SQLite FTS5

## Build + run

```bash
# Dev VM (the primary feedback loop)
nix build .#nixosConfigurations.osmoda-dev.config.system.build.vm
./result/bin/run-osmoda-dev-vm -m 4096 -smp 4

# ISO
nix build .#nixosConfigurations.osmoda-iso.config.system.build.isoImage

# Validate
nix flake check

# Rust
cargo check --workspace
cargo test --workspace

# agentd standalone (development)
cargo run -p agentd -- --socket /tmp/agentd.sock --state-dir /tmp/osmoda
```

## Implementation order

1. **flake.nix** — all inputs, nixosConfigurations for vm + iso
2. **crates/agentd** — minimal: /health + /system/query(processes) + /events/log + hash chain
3. **nix/modules/osmoda.nix** — agentd + gateway as systemd services
4. **nix/hosts/dev-vm.nix** — Sway desktop, auto-login, gateway running
5. **packages/osmoda-bridge** — register system_query + system_health as OpenClaw tools
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
GET  /agent/card           → AgentCard (EIP-8004)
POST /agent/card/generate  { name, description, services[] } → AgentCard
GET  /receipts             ?type=...&since=...&limit=N → Receipt[]
POST /incident/create      { name } → IncidentWorkspace
POST /incident/{id}/step   { action, result } → IncidentWorkspace
GET  /incident/{id}        → IncidentWorkspace (with all steps)
GET  /incidents            ?status=open → IncidentWorkspace[]
POST /backup/create        → BackupCreateResponse { backup_id, path, size_bytes, created_at }
GET  /backup/list          → BackupInfo[] { backup_id, path, size_bytes, created_at }
POST /backup/restore       { backup_id } → BackupRestoreResponse { restored_from, status }
```

## osmoda-keyd API reference (socket: /run/osmoda/keyd.sock)

```
POST /wallet/create       { chain: "ethereum"|"solana", label: str } → { id, chain, address }
GET  /wallet/list          → WalletInfo[]
POST /wallet/sign          { wallet_id, payload: hex } → { signature: hex } (policy-gated)
POST /wallet/send          { wallet_id, to, amount } → { signed_tx: hex } (policy-gated, no broadcast)
POST /wallet/delete        { wallet_id } → { deleted: wallet_id }
GET  /health               → { wallet_count, policy_loaded }
```

## osmoda-watch API reference (socket: /run/osmoda/watch.sock)

```
POST /switch/begin         { plan, ttl_secs, health_checks[] } → { id, previous_generation }
GET  /switch/status/{id}   → SwitchSession
POST /switch/commit/{id}   → SwitchSession (committed)
POST /switch/rollback/{id} → SwitchSession (rolled back)
POST /watcher/add          { name, check, interval_secs, actions[] } → Watcher
GET  /watcher/list          → Watcher[]
DEL  /watcher/remove/{id}  → { removed }
GET  /health               → { active_switches, watchers }
```

## osmoda-routines API reference (socket: /run/osmoda/routines.sock)

```
POST /routine/add          { name, trigger, action } → Routine
GET  /routine/list          → Routine[]
DEL  /routine/remove/{id}  → { removed }
POST /routine/trigger/{id} → { status, output }
GET  /routine/history       → RoutineHistoryEntry[]
GET  /health               → { routine_count, enabled_count }
```

## osmoda-voice API reference (socket: /run/osmoda/voice.sock)

All processing is local. STT via whisper.cpp (MIT), TTS via piper-tts (MIT), audio via PipeWire.
No data leaves the machine. No cloud APIs. No tracking.

```
GET  /voice/status          → { listening, whisper_model_loaded, piper_model_loaded }
POST /voice/transcribe      { audio_path: "/path/to/file.wav" } → { text, duration_ms }
POST /voice/speak           { text: "Hello" } → { audio_path, duration_ms } (plays via pw-play)
POST /voice/record          { duration_secs?, transcribe? } → { audio_path, text?, transcribe_duration_ms? }
POST /voice/listen          { enabled: bool } → { listening, previous }
```

## osmoda-mesh API reference (socket: /run/osmoda/mesh.sock, TCP: port 18800)

P2P encrypted agent-to-agent communication. Noise_XX + X25519 + ML-KEM-768 (hybrid post-quantum).
No central server. Invite-based pairing. Ed25519 identity signatures.

```
POST /invite/create        { ttl_secs?: u64 } → { invite_code, expires_at }
POST /invite/accept        { invite_code } → { peer_id, status }
GET  /peers                → PeerInfo[]
GET  /peer/{id}            → PeerInfo (with connection detail)
POST /peer/{id}/send       { message: MeshMessage } → { delivered: bool }
DEL  /peer/{id}            → { disconnected: peer_id }
POST /identity/rotate      → { new_instance_id, new_pubkeys }
GET  /identity             → MeshPublicIdentity
GET  /health               → { peer_count, connected_count, identity_ready }
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
-- hash = SHA-256(id|ts|type|actor|payload|prev_hash)   -- pipe-delimited
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
Node.js `@zvec/zvec` in the osmoda-bridge plugin. agentd handles ledger + watchers,
the plugin handles vector search. No Rust FFI complexity.

### Ground truth
Markdown files remain source of truth (OpenClaw compatible).
ZVEC indexes are derived — always rebuildable from files.
Path: `/var/lib/osmoda/memory/`

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
