# osModa — Project Status

Honest assessment of what works, what's placeholder, and what's next.

Last updated: 2026-04-30

## Recent operational changes (2026-04-30)

| Area | Change | Why |
|---|---|---|
| Spec-kit | github/spec-kit baked into every spawn (uv + specify-cli + templates) | Joins canonical AI-coding-agent ecosystem (92K stars). Closes the YC "software factories" weak-fit gap — see docs/planning/SPEC-KIT-INTEGRATION.md. |
| MCP | New `spec_kit_init` + `spec_kit_run` tools (91→92) | Agent invokes spec-driven dev as audited tool calls; ledger captures every phase transition. |
| Skills | New `spec-driven-development` (19→20) | Heuristic — when to invoke spec-kit, the 8-step workflow, common pitfalls. |
| API | `GET /api/v1/spec-kit/projects` (Bearer) | External SaaS integrators can list per-server spec-driven projects without SSHing in. |
| Agent card | `spec_driven_development:true` + `spec_kit_version` capability flags | Discoverable by other agents via `/.well-known/agent-card.json`. |
| OpenAPI | Bumped 1.2.1 → **1.2.2** | New `Spec-Kit` tag + `SpecKitProject` schema + 1 path. Total 26 documented paths. |

## Recent operational changes (2026-04-24)

| Area | Change | Why |
|---|---|---|
| Provisioning | Cloud-init hard-pinned to `--skip-nixos` (Ubuntu only path) | nixos-infect path is known-broken (3 unfixable failure modes); was bricking spawned servers |
| Claude Code | Bumped `@anthropic-ai/claude-code` from `^0.2.0` (resolved to 0.2.126) to `^2.1.75` | 2 majors behind; flag set partly broken on customer servers |
| Driver flags | Removed `--bare` (flaky across 2.1.x patches), added `--strict-mcp-config` (stable) | Deterministic MCP isolation regardless of which 2.1.x patch lands |
| install.sh | Phase tracking + `report_failed()` callback with last 200 log lines | Stuck-install class of incidents was invisible to dashboard |
| Spawn watchdog | Cron flags any order without heartbeat 25 min after creation as `install_failed` | Safety net for kernel-reboot failures where install.sh trap can't fire |
| Dashboard UI | Failure-state panel with phase + log + Rebuild / Heartbeat / Refund buttons | Operators no longer need SSH to diagnose stuck installs |
| Spawn deploy.sh | `setsid` + explicit fd redirect to log file | Previous nohup-via-SSH was leaving fd 1/2 pointing at half-closed Unix sockets — log output silently dropped for hours |

## Maturity Levels

- **Solid**: Compiles, has tests, uses correct algorithms, handles edge cases
- **Functional**: Compiles and works but lacks tests or has known limitations
- **Scaffold**: Structure is there, compiles, but contains placeholder logic
- **Planned**: Designed but not yet implemented

---

## Summary

| Metric | Count |
|--------|-------|
| Rust crates | 10 (9 daemons + 1 CLI) |
| MCP tools (via osmoda-mcp-bridge) | 91 |
| Runtime drivers | 2 (claude-code, openclaw) |
| System skills | 20 |
| NixOS systemd services | 13 (agentd, gateway, keyd, watch, routines, voice, mesh, mcpd, teachd, egress, app-restore, cloudflared, tailscale-auth) |
| Spawn API version | 1.2.0 |
| osmoda-gateway version | 0.2.0 |

---

## Rust Crates

### agentd — System Bridge Daemon

| Component | Maturity | Notes |
|-----------|----------|-------|
| `/health` endpoint | **Solid** | Returns real sysinfo metrics |
| `/system/query` endpoint | **Solid** | Processes, disk, hostname, uptime |
| `/events/log` endpoint | **Solid** | Hash-chained SQLite ledger, filter by type/actor/limit |
| Hash-chain ledger | **Solid** | SHA-256 chain (pipe-delimited format), verifiable with agentctl |
| `/memory/ingest` | **Functional** | Stores events to ledger; semantic vector search not yet wired (M1) |
| `/memory/recall` | **Solid** | FTS5 BM25-ranked full-text search with Porter stemming; falls back to keyword scan if FTS5 fails |
| `/memory/store` | **Functional** | Stores to ledger; no vector indexing yet |
| `/memory/health` | **Functional** | Reports model status and collection size |
| `/agent/card` | **Solid** | Serves/generates EIP-8004 card; serialization roundtrip tested |
| `/receipts` | **Solid** | Queries ledger events as structured receipts |
| Incident workspaces | **Solid** | Dedicated SQLite tables (incidents + incident_steps), 4 tests |
| `/backup/create` | **Solid** | WAL checkpointing before copy, timestamped output |
| `/backup/list` | **Solid** | Lists backups with IDs, sizes, timestamps |
| Backup retention | **Solid** | 7-day retention with automatic pruning; 2 tests |
| Graceful shutdown | **Solid** | Handles SIGTERM/SIGINT with clean resource cleanup |
| Input validation | **Solid** | Path traversal rejection, payload size limits, type checking |
| Subprocess timeouts | **Solid** | All subprocess calls capped with configurable timeouts |
| `/system/discover` | **Solid** | Parses `ss -tlnp` + `systemctl list-units`, detects known service types, cross-references with sysinfo; 4 tests |
| FTS5 search | **Solid** | Porter stemming, BM25 ranking, auto-sync trigger, backfill migration; 5 tests |
| **Tests** | **48** | agent card, incidents, backup, hash chain, FTS5, discovery, memory recall, approval, sandbox, input validation |

### osmoda-keyd — Crypto Wallet Daemon

| Component | Maturity | Notes |
|-----------|----------|-------|
| ETH key generation | **Solid** | k256 ECDSA, proper Keccak-256 for address derivation, known-vector test |
| SOL key generation | **Solid** | ed25519-dalek, bs58 encoding, stores 32-byte secret only |
| AES-256-GCM encryption | **Solid** | Encrypt/decrypt roundtrip tested, 12-byte nonce prepended |
| Argon2id KDF | **Solid** | Master key derived via Argon2id (64 MiB, 3 iterations); raw key + salt stored separately |
| Key zeroization | **Solid** | Drop impl zeroizes master key + cached keys, temporaries zeroized inline |
| Sign/verify roundtrip | **Solid** | Both ETH and SOL sign+verify tests pass |
| Policy engine | **Solid** | Fixed-point decimal arithmetic (18 decimals, no float), daily limits, allowlists; 8 tests |
| Receipt logging | **Solid** | Logs to agentd with correct chain field; best-effort (non-blocking) |
| Wallet deletion | **Solid** | Removes key file, zeroizes cache, updates index; 2 tests |
| `/wallet/send` | **Scaffold** | Signs an intent string, NOT a real transaction; no RLP encoding |
| Socket authentication | **Known limitation** | File permissions only (0o600); no token-based auth |
| **Tests** | **35** | sign/verify ETH+SOL, keccak256, encryption, KDF consistency, decimal policy, delete, persistence, cache eviction, label limit, tx building |

### osmoda-watch — SafeSwitch + Watchers

| Component | Maturity | Notes |
|-----------|----------|-------|
| SwitchSession state machine | **Solid** | Probation → Committed / RolledBack; 3 tests |
| Health checks | **Functional** | SystemdUnit, TcpPort, HttpGet, Command — all execute real commands |
| Auto-rollback | **Functional** | Calls `nix-env --rollback` + `switch-to-configuration switch` |
| `/switch/begin` | **Functional** | Records session; caller must apply the NixOS change first (by design) |
| Watcher escalation | **Functional** | restart → rollback → notify ladder; retries tracked |
| Watcher persistence | **Solid** | Saved/loaded from JSON on disk; 2 tests |
| Probation loop | **Functional** | Checks every 5s, auto-commits or rollbacks on TTL expiry |
| Input validation | **Solid** | Command path validation, arg metachar rejection, unit name sanitization; 12 tests |
| **Tests** | **27** | state machine, persistence, health checks, input validation, fleet coordination, watcher roundtrip |

### osmoda-routines — Background Automation

| Component | Maturity | Notes |
|-----------|----------|-------|
| Cron parser | **Solid** | Supports `*/N`, ranges, comma-separated, literals; 6 tests |
| Scheduler loop | **Functional** | Ticks every 60s, runs due routines |
| HealthCheck action | **Functional** | Executes real `systemctl is-system-running` |
| ServiceMonitor action | **Functional** | Checks systemd units via `systemctl is-active` |
| LogScan action | **Functional** | Runs `journalctl` with priority filter |
| MemoryMaintenance | **Functional** | Fetches recent events from agentd, counts by type, stores summary |
| Command action | **Functional** | Executes arbitrary commands with validation |
| Webhook action | **Functional** | Executes via curl (needs network access from proxy) |
| Input validation | **Solid** | Command path validation, interpreter blocking, URL scheme validation |
| Persistence | **Solid** | Saves/loads routines as JSON; 2 tests |
| **Tests** | **17** | cron parser (6), persistence (2), validation (7), command timeout (1), defaults (1) |

### osmoda-voice — Voice Pipeline (100% Local)

All processing on-device. No cloud. No tracking. No data leaves the machine.

| Component | Maturity | Notes |
|-----------|----------|-------|
| STT (whisper.cpp) | **Functional** | Subprocess invocation, 16kHz mono WAV input, 4-thread inference |
| TTS (piper-tts) | **Functional** | Subprocess invocation, stdin text → WAV output, auto-play via pw-play |
| `/voice/status` | **Solid** | Reports listening state, model availability |
| `/voice/transcribe` | **Functional** | Accepts WAV path, returns text + duration; logs transcription to agentd /memory/ingest (best-effort) |
| `/voice/speak` | **Functional** | Accepts text, synthesizes + plays audio, auto-cleans cache |
| `/voice/record` | **Functional** | Records via PipeWire (pw-record), optional auto-transcribe |
| `/voice/listen` | **Functional** | Enable/disable listening state toggle |
| VAD (record_clip) | **Functional** | Fixed-duration recording via timeout + pw-record |
| VAD (record_segment) | **Functional** | Duration-controlled recording with timeout, for continuous use |
| NixOS service | **Functional** | systemd unit with whisper.cpp + piper-tts; requires PipeWire |
| **Tests** | **4** | STT missing binary, TTS missing binary, VAD record_clip, VAD record_segment |

### osmoda-mesh — P2P Encrypted Daemon

| Component | Maturity | Notes |
|-----------|----------|-------|
| Ed25519 identity | **Solid** | Signing key generation + persistence (0o600), zeroize on Drop; tested |
| X25519 static key | **Solid** | Generated via `snow::Builder`, saved with public key; tested |
| ML-KEM-768 keypair | **Solid** | FIPS 203 (via `ml-kem` crate), encapsulate/decapsulate roundtrip tested |
| instance_id | **Solid** | `hex(SHA-256(noise_static_pubkey))[..32]` — deterministic, content-addressed; tested |
| Identity signature | **Solid** | Ed25519 sign over canonical JSON; tampered-signature rejection tested |
| Noise_XX handshake | **Solid** | `snow` crate, 3-message XX (X25519/ChaChaPoly/BLAKE2s), in-memory pipe test |
| ML-KEM PQ exchange | **Solid** | Post-Noise encapsulation inside encrypted tunnel; both directions |
| Hybrid HKDF re-key | **Solid** | `HKDF-SHA256(noise_hash || mlkem_1 || mlkem_2, info="osMODA-mesh-v1")`; tested |
| TCP transport | **Functional** | Length-prefixed framing, `snow` encrypt/decrypt, connection state machine |
| Auto-reconnect | **Functional** | Exponential backoff: 1s → 2s → 4s → 8s → max 60s; tested |
| Invite codes | **Solid** | base64url-encoded JSON, TTL validation, roundtrip + expiry rejection tested |
| Peer storage | **Solid** | JSON persistence, ConnectionState enum, save/load tested |
| `/invite/create` | **Functional** | Generates invite with configurable TTL |
| `/invite/accept` | **Functional** | Decodes invite, connects to peer, runs handshake |
| `/peers` | **Functional** | Returns all known peers with connection state |
| `/peer/{id}/send` | **Functional** | Sends encrypted MeshMessage to connected peer |
| `/peer/{id}` DELETE | **Functional** | Graceful disconnect, updates state |
| `/identity/rotate` | **Functional** | Generates new keypairs, disconnects all peers (re-invite required) |
| `/identity` GET | **Solid** | Returns current MeshPublicIdentity |
| `/health` GET | **Solid** | peer_count, connected_count, identity_ready; tested |
| MeshMessage serde | **Solid** | 5 variants (3 deleted), Chat has room_id for group rooms; all roundtrip-tested |
| Wire framing | **Solid** | Length-prefixed encode/decode, empty payload edge case tested |
| Recv/dispatch loop | **Functional** | Spawned per-connection after handshake; dispatches Heartbeat, HealthReport, Alert, Chat (DM + room), PqExchange |
| Outbound connect | **Functional** | Spawned on invite/accept and reconnect; 3 retries with 0/5/15s backoff; 10s TCP timeout |
| Dead-peer detection | **Functional** | 30s health loop; heartbeat probe on stale peers (>90s); reconnects Disconnected peers with known endpoints |
| Group rooms | **Functional** | In-memory rooms with members + message history; room_id on Chat messages; 5 REST endpoints |
| Audit logging | **Functional** | Logs to agentd ledger: connect, disconnect, message send/receive, health reports, alerts, DMs, room messages |
| NixOS service | **Functional** | systemd unit, TCP 18800, hardening directives, state dir 0700 |
| **Tests** | **44** | identity, handshake, messages, chat DM + room_id, invite, peers, transport, rooms, gossip, reconnect |
| **Known limitation** | — | No persistent transport state across restarts — peers must re-invite after daemon restart |

### osmoda-egress — Egress Proxy

| Component | Maturity | Notes |
|-----------|----------|-------|
| HTTP CONNECT proxy | **Functional** | Domain allowlist, localhost-only binding |
| Capability tokens | **Planned** | Currently uses static allowlist, not per-request tokens |
| **Tests** | **0** | No tests |

### osmoda-mcpd — MCP Server Manager

| Component | Maturity | Notes |
|-----------|----------|-------|
| Server lifecycle (start/stop/restart) | **Functional** | Spawns child processes, monitors health, auto-restarts crashed servers |
| Config loading | **Solid** | Reads NixOS-generated JSON config, handles missing/invalid files gracefully |
| OpenClaw config generation | **Solid** | Generates MCP servers JSON for OpenClaw; tested with proxy and without |
| Health monitoring | **Functional** | 10-second check loop, detects exited processes, auto-restart with count tracking |
| Egress proxy injection | **Solid** | Injects HTTP_PROXY/HTTPS_PROXY for servers with allowedDomains |
| Secret file injection | **Functional** | Reads secret from disk, injects as env var; warns but doesn't fail on read error |
| Reload endpoint | **Functional** | Re-reads config, starts new servers, stops removed ones |
| Receipt logging | **Functional** | Logs start/stop/crash/restart events to agentd ledger (best-effort) |
| NixOS service | **Functional** | systemd unit, depends on agentd + egress |
| **Tests** | **8** | Config serde, OpenClaw config generation (3), status transitions, health response, server list entry, default transport |

### osmoda-teachd — System Learning & Self-Optimization

| Component | Maturity | Notes |
|-----------|----------|-------|
| OBSERVE loop (30s) | **Functional** | Collects CPU (/proc/stat), memory (/proc/meminfo), service (systemctl), journal (journalctl) observations |
| LEARN loop (5m) | **Functional** | Detects recurring failures, memory trends, anomaly spikes, CPU-service correlations |
| SKILLGEN loop (6h) | **Functional** | Detects repeated agent tool sequences across sessions, auto-generates SKILL.md files; 6 tests |
| Agent action logging | **Solid** | Logs every tool execution via POST /observe/action; 30-day retention with auto-pruning |
| Skill candidate detection | **Functional** | Finds contiguous 3-6 tool sequences appearing in 3+ sessions, deduplicates by 80% overlap |
| Skill execution tracking | **Functional** | Records success/failure per skill, computes success rate |
| Pattern detection | **Functional** | Confidence scoring; patterns above 0.7 auto-generate knowledge docs |
| Knowledge CRUD | **Solid** | SQLite storage, manual + auto-generated docs, tags and categories; 2 tests |
| TEACH API | **Solid** | Keyword-based retrieval with confidence boost, ~6000 char token budget cap; 2 tests |
| Optimizer (suggest) | **Functional** | Generates ServiceRestart and Sysctl suggestions from knowledge docs |
| Optimizer (apply) | **Functional** | Applies via SafeSwitch (POST to osmoda-watch), auto-rollback on failure |
| SQLite persistence | **Solid** | WAL mode, 5s busy timeout; observations, patterns, knowledge_docs, optimizations, agent_actions, skill_candidates, skill_executions tables |
| Observation pruning | **Solid** | 7-day retention with automatic cleanup; tested |
| Receipt logging | **Functional** | Logs pattern detection, knowledge CRUD, optimization lifecycle to agentd |
| NixOS service | **Functional** | systemd unit, depends on agentd, Restart=on-failure |
| **Tests** | **22** | Health/teach serde (2), learner (4: trend, recurring, anomaly), optimizer (2: suggest, approve), teacher (2: match, no-match), knowledge CRUD (5: observations, patterns, knowledge, optimizations, pruning), skillgen (7: slug, name, overlap, confidence, skill_md, path_traversal) |

### agentctl — CLI Tool

| Component | Maturity | Notes |
|-----------|----------|-------|
| `events` subcommand | **Functional** | Queries ledger over Unix socket |
| `verify-ledger` | **Functional** | Verifies hash chain integrity |
| **Tests** | **0** | No tests |

---

## TypeScript (osmoda-bridge)

| Component | Maturity | Notes |
|-----------|----------|-------|
| agentd-client (inline) | **Functional** | HTTP-over-Unix-socket client for agentd |
| keyd-client.ts | **Functional** | HTTP-over-Unix-socket client for keyd |
| watch-client.ts | **Functional** | HTTP-over-Unix-socket client for watch |
| routines-client.ts | **Functional** | HTTP-over-Unix-socket client for routines |
| voice-client.ts | **Functional** | HTTP-over-Unix-socket client with status, speak, transcribe, record, listen |
| mesh-client.ts | **Functional** | HTTP-over-Unix-socket client for mesh daemon |
| mcpd-client.ts | **Functional** | HTTP-over-Unix-socket client for mcpd |
| teachd-client.ts | **Functional** | HTTP-over-Unix-socket client for teachd |
| Tool registrations | **Functional** | **90 tools** registered. Not integration-tested against live daemons |

### Tool breakdown (90 total)

| Category | Count | Tools |
|----------|-------|-------|
| agentd | 6 | system_health, system_query, system_discover, event_log, memory_store, memory_recall |
| system | 4 | shell_exec, file_read, file_write, directory_list |
| systemd | 2 | service_status, journal_logs |
| network | 1 | network_info |
| wallet (keyd) | 7 | wallet_create, wallet_list, wallet_sign, wallet_send, wallet_delete, wallet_receipt, wallet_build_tx |
| switch (watch) | 4 | safe_switch_begin, safe_switch_status, safe_switch_commit, safe_switch_rollback |
| watcher (watch) | 2 | watcher_add, watcher_list |
| fleet (watch) | 4 | fleet_propose, fleet_status, fleet_vote, fleet_rollback |
| routine (routines) | 3 | routine_add, routine_list, routine_trigger |
| identity (agentd) | 1 | agent_card |
| receipt (agentd) | 3 | receipt_list, incident_create, incident_step |
| voice | 5 | voice_status, voice_speak, voice_transcribe, voice_record, voice_listen |
| backup (agentd) | 2 | backup_create, backup_list |
| mesh | 11 | mesh_identity, mesh_invite_create, mesh_invite_accept, mesh_peers, mesh_peer_send, mesh_peer_disconnect, mesh_health, mesh_room_create, mesh_room_join, mesh_room_send, mesh_room_history |
| mcp (mcpd) | 4 | mcp_servers, mcp_server_start, mcp_server_stop, mcp_server_restart |
| teach (teachd) | 14 | teach_status, teach_observations, teach_patterns, teach_knowledge, teach_knowledge_create, teach_context, teach_optimize_suggest, teach_optimize_apply, teach_skill_candidates, teach_skill_generate, teach_skill_promote, teach_observe_action, teach_skill_execution, teach_skill_detect |
| approval (agentd) | 4 | approval_request, approval_pending, approval_approve, approval_check |
| sandbox (agentd) | 2 | sandbox_exec, capability_mint |
| app (direct) | 6 | app_deploy, app_list, app_logs, app_stop, app_restart, app_remove |
| safety | 4 | safety_rollback, safety_status, safety_panic, safety_restart |

---

## App Management (Bridge Tools)

App process management via `systemd-run` transient units. No new Rust daemon — 6 bridge tools call systemd directly. JSON registry provides boot persistence.

| Component | Maturity | Notes |
|-----------|----------|-------|
| `app_deploy` | **Functional** | systemd-run with DynamicUser isolation, resource limits, env vars |
| `app_list` | **Functional** | Reads registry + live systemctl show for each app |
| `app_logs` | **Functional** | journalctl wrapper with unit filter |
| `app_stop` | **Functional** | systemctl stop + registry status update |
| `app_restart` | **Functional** | systemctl restart or re-deploy from registry if inactive |
| `app_remove` | **Functional** | Stop + delete from registry |
| Boot persistence | **Functional** | JSON registry + oneshot restore service re-creates transient units on boot |
| Input validation | **Solid** | Name sanitization, absolute path check, restart policy validation, env key sanitization |

---

## NixOS Integration

| Component | Maturity | Notes |
|-----------|----------|-------|
| osmoda.nix module | **Functional** | Options + 12 systemd services + channels + mesh + mcpd + teachd + remote access defined |
| osmoda-agentd service | **Functional** | Runs as root, state dir at /var/lib/osmoda |
| osmoda-keyd service | **Functional** | PrivateNetwork=true, RestrictAddressFamilies=AF_UNIX |
| osmoda-watch service | **Functional** | Runs as root (needs nixos-rebuild access) |
| osmoda-routines service | **Functional** | systemd hardening applied |
| osmoda-voice service | **Functional** | Requires PipeWire for audio I/O |
| osmoda-mesh service | **Functional** | TCP 18800, systemd hardening, state dir 0700 |
| osmoda-mcpd service | **Functional** | MCP server lifecycle, depends on agentd + egress |
| osmoda-teachd service | **Functional** | System learning, depends on agentd, Restart=on-failure |
| osmoda-egress service | **Functional** | DynamicUser, domain-filtered proxy |
| Multi-agent routing | **Functional** | `osmoda` (Opus, full) + `mobile` (Sonnet, full access, concise) agents with channel bindings |
| OpenClaw gateway service | **Functional** | Depends on agentd, multi-agent config generated from NixOS options |
| Channel config (Telegram) | **Functional** | `channels.telegram.enable`, botTokenFile, allowedUsers |
| Channel config (WhatsApp) | **Functional** | `channels.whatsapp.enable`, credentialDir, allowedNumbers |
| Remote access (Cloudflare) | **Functional** | `remoteAccess.cloudflare.enable`, quick tunnel or credentialed, systemd service |
| Remote access (Tailscale) | **Functional** | `remoteAccess.tailscale.enable`, auto-auth oneshot, forwards to NixOS built-in |
| Firewall rules | **Functional** | Mesh port (18800) opened conditionally when mesh.enable = true |
| flake.nix overlays | **Functional** | 10 Rust packages built via crane |
| dev-vm.nix | **Functional** | QEMU VM with Sway desktop |
| iso.nix | **Functional** | Installer ISO config |
| server.nix | **Functional** | Headless server config |

---

## Messaging Channels

| Component | Maturity | Notes |
|-----------|----------|-------|
| Telegram NixOS options | **Functional** | `channels.telegram.enable`, botTokenFile, allowedUsers |
| WhatsApp NixOS options | **Functional** | `channels.whatsapp.enable`, credentialDir, allowedNumbers |
| Config file generation | **Functional** | Generates OpenClaw config JSON with channel settings, passed via `--config` |
| Credential management | **Functional** | Activation script creates + secures secrets dir and WhatsApp credential dir |
| Actual channel connections | **Depends on OpenClaw** | osModa generates config; OpenClaw runs the Telegram/WhatsApp adapters |

---

## Known Limitations

1. **No real transaction building**: `wallet/send` signs an intent string, not an RLP-encoded ETH transaction or a Solana transaction. Broadcasting requires external tooling.

2. **No network from keyd**: By design. keyd has `PrivateNetwork=true`. Signed transactions must be broadcast by the caller.

3. **Memory system is M0**: Semantic vector search is not yet wired. Memory recall uses FTS5 BM25-ranked full-text search (with keyword fallback). Semantic search (usearch + fastembed) deferred to M1.

4. **SafeSwitch doesn't execute the change**: `switch/begin` records the session but the caller must apply the NixOS change. The daemon manages the health-check/rollback lifecycle after the change.

5. **No end-to-end integration tests**: Each crate has unit tests. No tests verify the full daemon-to-daemon-to-bridge pipeline.

6. **Socket auth is file-permissions only**: No token-based auth for Unix socket access. Relies on filesystem permissions (all sockets 0o600 owner-only) + `umask(0o077)` enforced at daemon startup (since 2026-02-27).

7. **Mesh peers don't survive restarts**: No persistent transport state. Peers must re-invite after daemon restart. Identity and peer metadata persist, but active connections do not.

8. **Voice requires PipeWire**: STT/TTS work but recording/playback needs PipeWire running. Headless servers without audio won't use voice.

---

## Test Coverage

```
cargo test --workspace
```

| Crate | Tests | What's tested |
|-------|-------|---------------|
| agentd | 48 | Agent card, incidents (5), backup pruning (2), hash chain (4), FTS5 search (5), service discovery (4), memory recall (2), approval (4), sandbox (4), input validation (18) |
| osmoda-keyd | 35 | ETH+SOL sign/verify, keccak256 vector, encryption roundtrip, Argon2 KDF, decimal policy (8), wallet delete (2), persistence, cache eviction, label limit, tx building (10) |
| osmoda-watch | 27 | Switch state machine (3), watcher persistence (2), health check serde, input validation (12), fleet coordination (9) |
| osmoda-routines | 17 | Cron parser (6), persistence (2), validation (7), command timeout, defaults |
| osmoda-voice | 4 | STT missing binary, TTS missing binary, VAD record_clip, VAD record_segment |
| osmoda-mesh | 44 | Identity (5), Noise_XX handshake+transport+HKDF (3), message serde (7), chat DM+room_id (2), invite (3), peers (3), reconnect (2), rooms (3), gossip (3), transport (5), health (3), wire framing (5) |
| osmoda-mcpd | 8 | Config serde, OpenClaw config generation (3), status transitions, health response, server list entry, default transport |
| osmoda-teachd | 22 | Health/teach serde (2), learner (4), optimizer (2), teacher (2), knowledge CRUD (5), skillgen (7: slug, name, overlap, confidence, skill_md, path_traversal) |
| agentctl | 0 | — |
| osmoda-egress | 0 | — |
| **Total** | **205** | **All pass** |

---

## spawn.os.moda — Hosted Provisioning

Separate private repo. Not part of the open source OS. Visit [spawn.os.moda](https://spawn.os.moda) to deploy a managed osModa server.

### Server Detail Dashboard (dashboard.html)

Redesigned single-column layout with tabbed interface (Overview / Chat / Settings).

| Component | Maturity | Notes |
|-----------|----------|-------|
| Header | **Functional** | Bigger server name (20px), subtitle line (plan + location + price), pill-shaped status badge |
| Overview tab | **Functional** | Single-column layout, prominent agent card, orchestration cards, 2-col channel cards, system + settings grid, collapsible sections |
| Automation card | **Functional** | Shows active routines (interval, last-run, status) and health watchers (check type, interval, result) from heartbeat |
| Activity feed card | **Functional** | 15 most recent agentd audit log events with timestamp, type, and actor |
| Intelligence card | **Functional** | TeachD stats (observations, patterns, knowledge docs) + detected patterns with confidence scores; conditional |
| Tool servers card | **Functional** | MCP server list with status, PID, uptime; conditional |
| Chat tab | **Functional** | Horizontal activity bar (replaces old sidebar), Claude-like rounded input with circular send button, no-bubble agent messages, user messages as accent bubbles, activity dropdown, markdown rendering (code blocks, lists, headers, links, blockquotes) |
| Markdown rendering | **Functional** | Fenced code blocks with syntax highlighting, inline code, headers, bold/italic, ordered/unordered lists, links, blockquotes |
| Responsive layout | **Functional** | Removed right sidebar column entirely — everything single-column flow |

### v1 Programmatic API

Agent-to-agent spawning API with x402 payment gating (Coinbase standard).
**v1.2.0** (2026-04-18): modular runtime + per-server credentials/agents management —
see `apps/spawn/CHANGELOG.md`. **v1.1.0** (2026-04-17): idempotency, structured errors,
token lifecycle, WS hardening.

| Component | Maturity | Notes |
|-----------|----------|-------|
| Spawn runtime/credentials at request time | **Functional** | `POST /api/v1/spawn/:planId` accepts `{runtime, credentials[], default_model}`; cloud-init passes them to install.sh. |
| Per-server Engine tab (dashboard) | **Functional** | Lists drivers / credentials / agents; CRUD + test via spawn-app proxy → SSH → customer gateway. |
| Proxy endpoints `/api/dashboard/servers/:id/config/*` | **Functional** | GET/PUT/PATCH/DELETE for agents + credentials + drivers. |

| Component | Maturity | Notes |
|-----------|----------|-------|
| Agent Card (`/.well-known/agent-card.json`) | **Solid** | A2A + ERC-8004 (protocols array, chainId per payment method, semver 1.1.0) |
| `GET /api/v1/plans` | **Solid** | Plan list with x402 pricing, regions, network mode |
| `POST /api/v1/spawn/:planId` | **Solid** | x402-gated spawn. **Idempotency-Key** pre-check runs BEFORE x402 middleware → retries never re-pay |
| `GET /api/v1/status/:orderId` | **Solid** | Basic status free; full details require Bearer `osk_`; enforces token expiry/revoke |
| `GET /api/v1/tokens/:token_id` | **Solid** | Token metadata (own-token only) |
| `DELETE /api/v1/tokens/:token_id` | **Solid** | Token revoke (own-token only); `204` on success |
| `WS /api/v1/chat/:orderId` | **Solid** | 30 s heartbeat, 10 min idle close (4003), enforced backpressure (drops paused), 3 sessions/token cap |
| `GET /api/v1/docs` | **Solid** | OpenAPI 3.0.3 v1.1.0 — `securitySchemes.bearerAuth`, `Error` schema, `required` arrays, examples, `x-websocket` |
| x402 payment middleware | **Functional** | `@x402/express` + `@x402/evm` + `@x402/svm` + `@x402/core`, USDC on Base (EVM) + Solana (SVM) |
| Structured error envelope | **Solid** | `{code, message, detail?, request_id, error}` on every /api/v1/* + agent-card error; legacy `error` kept one release |
| Request IDs | **Solid** | `X-Request-Id: req_<ulid>` on every response, prefixed into `[req_…]` log lines |
| Token lifecycle | **Solid** | `tokens.enc` AES-256-GCM store; 1-year default TTL; lazy metadata for legacy tokens |
| Per-token rate limits | **Solid** | spawn 10/h, status 120/min, chat 3 concurrent — all with `Retry-After` on 429 |
| `@osmoda/client` TypeScript SDK | **Functional** | `packages/osmoda-client/` — handwritten to match `/api/v1/docs`; typechecks clean |
| Agent skill doc (`/SKILL.md`) | **Functional** | 369-line plain-text agent-readable doc with full API reference, x402 flow, all 90 tools |

### Heartbeat Pipeline

| Component | Maturity | Notes |
|-----------|----------|-------|
| System health | **Functional** | CPU, RAM, disk, uptime from agentd |
| Agent instances | **Functional** | Name + status from OpenClaw agent dirs |
| Daemon health | **Functional** | 10 daemons: active/pid per daemon |
| Mesh identity + peers | **Functional** | Instance ID, connected peers |
| Routines | **Functional** | Active routines with trigger, interval, last-run from routines daemon |
| Routine history | **Functional** | Recent execution history (status, output) |
| Watchers | **Functional** | Health watchers with check type, interval, status from watch daemon |
| Recent events | **Functional** | 30 most recent agentd audit log events |
| TeachD health | **Functional** | Observation/pattern/knowledge/optimization counts, loop status |
| TeachD patterns | **Functional** | Top 10 high-confidence patterns (>0.7) |
| MCP servers | **Functional** | Server list with name, status, PID, uptime from mcpd |
| SafeSwitch sessions | **Functional** | Recent switch sessions (id, plan, status, health checks) from watch daemon |
| NixOS generation | **Functional** | Current NixOS system generation path from /nix/var/nix/profiles/system |

---

## Security Hardening (2026-02-26)

All items verified by automated pentest on live server.

| Fix | Severity | Status |
|-----|----------|--------|
| Socket permissions 0o660 → 0o600 (watch, routines, mesh, mcpd, teachd) | HIGH | Done |
| Mesh TCP default bind 0.0.0.0 → 127.0.0.1 | CRITICAL | Done |
| shell_exec: block dangerous commands (was warn-only) | CRITICAL | Done |
| shell_exec: expanded blocklist (7 → 17 patterns) | CRITICAL | Done |
| directory_list: add validateFilePath() | CRITICAL | Done |
| agentd error responses: generic JSON (no stack trace leak) | CRITICAL | Done |
| NixOS module: ProtectSystem=strict, ProtectHome, NoNewPrivileges, PrivateTmp, RestrictSUIDSGID on routines/mesh/mcpd/teachd | MEDIUM | Done |
| NixOS module: RestrictAddressFamilies on mesh (AF_UNIX + AF_INET + AF_INET6) | MEDIUM | Done |
| NixOS module: mesh listenAddr default 0.0.0.0 → 127.0.0.1 | MEDIUM | Done |
| `umask(0o077)` enforced at startup in all 9 daemons | HIGH | Done |
| `DefaultBodyLimit` added to all 8 socket daemons | MEDIUM | Done |
| keyd policy counters persisted to disk (counters.json) | MEDIUM | Done |
| Mesh single-use invite enforcement (409 on replay) | MEDIUM | Done |
| Mesh per-IP TCP rate limiting (5/60s) | MEDIUM | Done |
| Bridge `shell_exec` rate limiting (30/60s) | MEDIUM | Done |
| Bridge `file_read` size cap (10 MiB) | MEDIUM | Done |
| Bridge symlink escape prevention in `validateFilePath()` | MEDIUM | Done |

### Pentest results (2026-02-27, post-hardening)

```
Socket permissions:    7/7 PASS (all 0600)
Mesh bind address:     PASS (127.0.0.1:18800)
Network exposure:      PASS (only SSH + nginx exposed)
Daemon health:         7/7 PASS (headless; voice + egress skip on servers without audio/sandbox)
Injection attacks:     3/3 PASS (SQL injection, path traversal, shell injection)
Payload bombs:         PASS (agentd survived 1MB payload)
Error hardening:       PASS (no stack trace leak)
Data preservation:     PASS (teachd observations, keyd policy, all persistent state)
Hash chain integrity:  PASS (321 events, all valid, zero broken chain links)
Rate limiting:         PASS (all public endpoints enforce rate limits)
umask enforcement:     PASS (all 9 daemons call umask(0o077) at startup)
Body size limits:      PASS (all 8 socket daemons have DefaultBodyLimit)
Stress test:           PASS (700/700 concurrent health checks, 50 concurrent queries)
```

### Remaining known issues

- F-3: Unbounded `Vec<RoomMessage>` in mesh rooms (memory growth)
- F-5: No agentd ledger pruning (grows forever)
- F-6: osmoda-egress has zero tests

### Resolved since last pentest

- ~~F-1: No `RequestBodyLimit` middleware~~ → All 8 daemons now have `DefaultBodyLimit::max()` (1 MiB for most, 256 KiB for voice)
- ~~F-7: keyd daily policy counters in-memory only~~ → Counters now persist to `counters.json` on disk; survive daemon restarts

---

## What's Next

1. **Approval gate for destructive ops** — code-enforced confirmation before destructive operations (currently convention-based via agent prompt, not runtime-enforced). This is the #1 safety priority.
2. **Tier 1/Tier 2 sandbox implementation** — enforce the trust tier model with bubblewrap isolation + egress proxy for third-party tools
3. **End-to-end VM test** — boot the dev VM, verify all daemons start and communicate
4. **Integration tests** — bridge → daemon → ledger pipeline tests
5. **Wire semantic memory** — connect usearch + fastembed so `memory/recall` returns hybrid BM25 + vector results
6. **Token-based socket auth** — capability tokens for fine-grained access control
7. **Persistent mesh sessions** — save/restore transport state across daemon restarts
8. **External security audit** — independent review of mesh crypto (Noise_XX + ML-KEM-768)
9. **Real transaction building** — RLP encoding for ETH, Solana transaction structs (lower priority — not the core value prop)
10. ~~**Web dashboard with live chat**~~ — DONE. Redesigned detail page: single-column layout, tabbed Overview/Chat/Settings, markdown rendering in chat, horizontal activity bar, collapsible sections
