# osModa — Project Status

Honest assessment of what works, what's placeholder, and what's next.

Last updated: 2026-02-24

## Maturity Levels

- **Solid**: Compiles, has tests, uses correct algorithms, handles edge cases
- **Functional**: Compiles and works but lacks tests or has known limitations
- **Scaffold**: Structure is there, compiles, but contains placeholder logic
- **Planned**: Designed but not yet implemented

---

## Summary

| Metric | Count |
|--------|-------|
| Rust crates | 9 (8 daemons + 1 CLI) |
| Tests passing | 121 |
| Bridge tools registered | 58 |
| System skills | 15 |
| NixOS systemd services | 11 (agentd, gateway, keyd, watch, routines, voice, mesh, mcpd, egress, cloudflared, tailscale-auth) |

---

## Rust Crates

### agentd — Kernel Bridge Daemon

| Component | Maturity | Notes |
|-----------|----------|-------|
| `/health` endpoint | **Solid** | Returns real sysinfo metrics |
| `/system/query` endpoint | **Solid** | Processes, disk, hostname, uptime |
| `/events/log` endpoint | **Solid** | Hash-chained SQLite ledger, filter by type/actor/limit |
| Hash-chain ledger | **Solid** | SHA-256 chain (pipe-delimited format), verifiable with agentctl |
| `/memory/ingest` | **Functional** | Stores events to ledger; ZVEC vector search not yet wired |
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
| **Tests** | **20** | agent card, incidents (5), backup (2), hash chain (4), FTS5 (5), discovery (4) |

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
| **Tests** | **21** | sign/verify ETH+SOL, keccak256, encryption, KDF consistency, decimal policy (8), delete, persistence, cache eviction, label limit |

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
| **Tests** | **18** | state machine (3), persistence (2), health check serialization, input validation (12) |

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
| **Tests** | **31** | identity (5), handshake (3), messages (7 + chat DM + room_id), invite (3), peers (3), transport (2), rooms (3) |
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
| Tool registrations | **Functional** | **58 tools** registered. Not integration-tested against live daemons |

### Tool breakdown (58 total)

| Category | Count | Tools |
|----------|-------|-------|
| agentd | 6 | system_health, system_query, system_discover, event_log, memory_store, memory_recall |
| system | 4 | shell_exec, file_read, file_write, directory_list |
| systemd | 2 | service_status, journal_logs |
| network | 1 | network_info |
| wallet (keyd) | 6 | wallet_create, wallet_list, wallet_sign, wallet_send, wallet_delete, wallet_receipt |
| switch (watch) | 4 | safe_switch_begin, safe_switch_status, safe_switch_commit, safe_switch_rollback |
| watcher (watch) | 2 | watcher_add, watcher_list |
| routine (routines) | 3 | routine_add, routine_list, routine_trigger |
| identity (agentd) | 1 | agent_card |
| receipt (agentd) | 3 | receipt_list, incident_create, incident_step |
| voice | 5 | voice_status, voice_speak, voice_transcribe, voice_record, voice_listen |
| backup (agentd) | 2 | backup_create, backup_list |
| mesh | 11 | mesh_identity, mesh_invite_create, mesh_invite_accept, mesh_peers, mesh_peer_send, mesh_peer_disconnect, mesh_health, mesh_room_create, mesh_room_join, mesh_room_send, mesh_room_history |
| mcp (mcpd) | 4 | mcp_servers, mcp_server_start, mcp_server_stop, mcp_server_restart |
| safety | 4 | safety_rollback, safety_status, safety_panic, safety_restart |

---

## NixOS Integration

| Component | Maturity | Notes |
|-----------|----------|-------|
| osmoda.nix module | **Functional** | Options + 11 systemd services + channels + mesh + mcpd + remote access defined |
| osmoda-agentd service | **Functional** | Runs as root, state dir at /var/lib/osmoda |
| osmoda-keyd service | **Functional** | PrivateNetwork=true, RestrictAddressFamilies=AF_UNIX |
| osmoda-watch service | **Functional** | Runs as root (needs nixos-rebuild access) |
| osmoda-routines service | **Functional** | systemd hardening applied |
| osmoda-voice service | **Functional** | Requires PipeWire for audio I/O |
| osmoda-mesh service | **Functional** | TCP 18800, systemd hardening, state dir 0700 |
| osmoda-mcpd service | **Functional** | MCP server lifecycle, depends on agentd + egress |
| osmoda-egress service | **Functional** | DynamicUser, domain-filtered proxy |
| OpenClaw gateway service | **Functional** | Depends on agentd, config file generated from NixOS options |
| Channel config (Telegram) | **Functional** | `channels.telegram.enable`, botTokenFile, allowedUsers |
| Channel config (WhatsApp) | **Functional** | `channels.whatsapp.enable`, credentialDir, allowedNumbers |
| Remote access (Cloudflare) | **Functional** | `remoteAccess.cloudflare.enable`, quick tunnel or credentialed, systemd service |
| Remote access (Tailscale) | **Functional** | `remoteAccess.tailscale.enable`, auto-auth oneshot, forwards to NixOS built-in |
| Firewall rules | **Functional** | Mesh port (18800) opened conditionally when mesh.enable = true |
| flake.nix overlays | **Functional** | 9 Rust packages built via crane |
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

3. **Memory system is M0**: ZVEC vector search is not wired. Memory recall uses FTS5 BM25-ranked full-text search (with keyword fallback). Semantic search deferred to M1.

4. **SafeSwitch doesn't execute the change**: `switch/begin` records the session but the caller must apply the NixOS change. The daemon manages the health-check/rollback lifecycle after the change.

5. **No end-to-end integration tests**: Each crate has unit tests. No tests verify the full daemon-to-daemon-to-bridge pipeline.

6. **Socket auth is file-permissions only**: No token-based auth for Unix socket access. Relies on filesystem permissions (0600/0660).

7. **Mesh peers don't survive restarts**: No persistent transport state. Peers must re-invite after daemon restart. Identity and peer metadata persist, but active connections do not.

8. **Voice requires PipeWire**: STT/TTS work but recording/playback needs PipeWire running. Headless servers without audio won't use voice.

---

## Test Coverage

```
cargo test --workspace
```

| Crate | Tests | What's tested |
|-------|-------|---------------|
| agentd | 22 | Agent card, incidents (5), backup pruning (2), hash chain (4), FTS5 search (5), service discovery (4), memory recall (2) |
| osmoda-keyd | 21 | ETH+SOL sign/verify, keccak256 vector, encryption roundtrip, Argon2 KDF, decimal policy (8 tests), wallet delete, persistence, cache eviction, label limit |
| osmoda-watch | 18 | Switch state machine (3), watcher persistence (2), health check serde, input validation (12) |
| osmoda-routines | 17 | Cron parser (6), persistence (2), validation (7), command timeout, defaults |
| osmoda-voice | 4 | STT missing binary, TTS missing binary, VAD record_clip, VAD record_segment |
| osmoda-mesh | 31 | Identity gen/load/verify/tamper (5), Noise_XX handshake+transport+HKDF (3), message serde (7), chat DM+room_id (2), invite roundtrip/expiry/invalid (3), peers persistence (3), reconnect backoff (2), room create/join/history (3) |
| osmoda-mcpd | 8 | Config serde, OpenClaw config generation (3), status transitions, health response, server list entry, default transport |
| agentctl | 0 | — |
| osmoda-egress | 0 | — |
| **Total** | **121** | **All pass** |

---

## spawn.os.moda — Provisioning Service

Separate private repo (`apps/spawn/`). Not part of the open source OS.

| Component | Maturity | Notes |
|-----------|----------|-------|
| Landing page (index.html) | **Solid** | Spirit orb, plan selection, x402 payments, responsive |
| x402 USDC payments (Base) | **Solid** | MetaMask flow, on-chain verification, replay prevention |
| x402 USDC payments (Solana) | **Solid** | Phantom flow, SPL token transfer, ATA creation |
| Hetzner provisioning | **Functional** | Creates server, passes cloud-init; needs end-to-end testing |
| Lead capture | **Solid** | Email + plan, AES-256-GCM encrypted storage, honeypot, rate limits |
| Cloud-init with order_id | **Solid** | Passes order_id + callback URL to install.sh |
| `POST /api/heartbeat` | **Solid** | Rate-limited, validates order_id, stores health data, promotes status |
| `GET /api/status/:id` | **Solid** | Returns plan details, health, computed SSH/tunnel commands |
| Management dashboard | **Solid** | Order lookup, status card, health metrics, quick actions, upsells, auto-refresh |
| Heartbeat systemd timer | **Solid** | In install.sh; 30s after boot, then every 5 min |
| Phone-home on install | **Solid** | Curls callback URL on install completion |

### Spawn architecture

```
User pays USDC → spawn.os.moda → Hetzner API (create server)
                                       ↓
                              cloud-init runs install.sh
                              --order-id UUID --callback-url URL
                                       ↓
                              install completes → phone home
                                       ↓
                              heartbeat timer (every 5 min)
                              sends: status, cpu, ram, disk
                                       ↓
                              /manage?id=UUID → dashboard
```

---

## What's Next

1. **Wire ZVEC memory** — connect the vector search so `memory/recall` returns real results
2. **Real transaction building** — RLP encoding for ETH, Solana transaction structs
3. **End-to-end VM test** — boot the dev VM, verify all daemons start and communicate
4. **Integration tests** — bridge → daemon → ledger pipeline tests
5. **Token-based socket auth** — capability tokens for fine-grained access control
6. **Persistent mesh sessions** — save/restore transport state across daemon restarts
7. **NixOS-native deploy** — replace imperative nohup/iptables with osmoda.nix module on Hetzner
8. **Telegram bot** — `/start <order_id>` for server management via Telegram
9. **Email notifications** — send order_id + manage link after spawn
10. **Plan upgrades** — resize Hetzner server via management dashboard
