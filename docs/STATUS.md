# osModa — Project Status

Honest assessment of what works, what's placeholder, and what's next.

Last updated: 2026-02-23

## Maturity Levels

- **Solid**: Compiles, has tests, uses correct algorithms, handles edge cases
- **Functional**: Compiles and works but lacks tests or has known limitations
- **Scaffold**: Structure is there, compiles, but contains placeholder logic
- **Planned**: Designed but not yet implemented

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
| `/memory/recall` | **Scaffold** | Returns empty results — ZVEC binding deferred to M1 |
| `/memory/store` | **Functional** | Stores to ledger; no vector indexing yet |
| `/agent/card` | **Solid** | Serves/generates EIP-8004 card; serialization roundtrip tested |
| `/receipts` | **Solid** | Queries ledger events as structured receipts |
| Incident workspaces | **Solid** | Dedicated SQLite tables (incidents + incident_steps), 4 tests |
| Backup system | **Solid** | Daily timer, 7-day retention, WAL checkpointing before backup |
| Graceful shutdown | **Solid** | All daemons handle SIGTERM/SIGINT with clean resource cleanup |
| Input validation | **Solid** | Path traversal rejection, payload size limits, type checking |
| Subprocess timeouts | **Solid** | All subprocess calls capped with configurable timeouts |

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

### Messaging Channels (NixOS Config)

| Component | Maturity | Notes |
|-----------|----------|-------|
| Telegram NixOS options | **Functional** | `channels.telegram.enable`, botTokenFile, allowedUsers |
| WhatsApp NixOS options | **Functional** | `channels.whatsapp.enable`, credentialDir, allowedNumbers |
| Config file generation | **Functional** | Generates OpenClaw config JSON with channel settings, passed via `--config` |
| Credential management | **Functional** | Activation script creates + secures secrets dir and WhatsApp credential dir |
| Actual channel connections | **Depends on OpenClaw** | osModa generates config; OpenClaw runs the Telegram/WhatsApp adapters |

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

### osmoda-routines — Background Automation

| Component | Maturity | Notes |
|-----------|----------|-------|
| Cron parser | **Solid** | Supports `*/N`, ranges, comma-separated, literals; 6 tests |
| Scheduler loop | **Functional** | Ticks every 60s, runs due routines |
| HealthCheck action | **Functional** | Executes real `systemctl is-system-running` |
| ServiceMonitor action | **Functional** | Checks systemd units via `systemctl is-active` |
| LogScan action | **Functional** | Runs `journalctl` with priority filter |
| MemoryMaintenance | **Functional** | Fetches recent events from agentd, counts by type, stores summary |
| Command action | **Functional** | Executes arbitrary commands |
| Webhook action | **Functional** | Executes via curl (needs network access from proxy) |
| Persistence | **Solid** | Saves/loads routines as JSON; 2 tests |

### osmoda-voice — Voice Pipeline (100% Local, Open Source)

All processing on-device. No cloud. No tracking. No data leaves the machine.

| Component | Maturity | Notes |
|-----------|----------|-------|
| STT (whisper.cpp) | **Functional** | Subprocess invocation, 16kHz mono WAV input, 4-thread inference |
| TTS (piper-tts) | **Functional** | Subprocess invocation, stdin text → WAV output, auto-play via pw-play |
| `/voice/status` | **Solid** | Reports listening state, model availability |
| `/voice/transcribe` | **Functional** | Accepts WAV path, returns text + duration |
| `/voice/speak` | **Functional** | Accepts text, synthesizes + plays audio, auto-cleans cache |
| `/voice/record` | **Functional** | Records via PipeWire (pw-record), optional auto-transcribe |
| `/voice/listen` | **Functional** | Enable/disable listening state toggle |
| VAD (record_clip) | **Functional** | Fixed-duration recording via timeout + pw-record |
| VAD (record_segment) | **Functional** | Duration-controlled recording with timeout, for continuous use |
| NixOS service | **Functional** | systemd unit with whisper.cpp + piper-tts; requires PipeWire (systemd `Requires=` dependency) |

### osmoda-egress — Egress Proxy

| Component | Maturity | Notes |
|-----------|----------|-------|
| HTTP CONNECT proxy | **Functional** | Domain allowlist, localhost-only binding |
| Capability tokens | **Planned** | Currently uses static allowlist, not per-request tokens |

### agentctl — CLI Tool

| Component | Maturity | Notes |
|-----------|----------|-------|
| `events` subcommand | **Functional** | Queries ledger over Unix socket |
| `verify-ledger` | **Functional** | Verifies hash chain integrity |

---

## TypeScript (osmoda-bridge)

| Component | Maturity | Notes |
|-----------|----------|-------|
| agentd-client.ts | **Functional** | HTTP-over-Unix-socket client |
| keyd-client.ts | **Functional** | HTTP-over-Unix-socket client |
| watch-client.ts | **Functional** | HTTP-over-Unix-socket client |
| routines-client.ts | **Functional** | HTTP-over-Unix-socket client |
| 37 tool registrations | **Functional** | All registered (incl. backup_create, backup_list); not integration-tested against live daemons |
| Voice client | **Functional** | HTTP-over-Unix-socket client with status, speak, transcribe, record, listen |

---

## NixOS Integration

| Component | Maturity | Notes |
|-----------|----------|-------|
| osmoda.nix module | **Functional** | Options + 7 systemd services + Telegram/WhatsApp channels defined |
| osmoda-keyd service | **Functional** | PrivateNetwork=true, RestrictAddressFamilies=AF_UNIX |
| osmoda-watch service | **Functional** | Runs as root (needs nixos-rebuild access) |
| osmoda-routines service | **Functional** | systemd hardening applied |
| flake.nix overlays | **Functional** | 6 Rust packages built via crane |
| dev-vm.nix | **Functional** | QEMU VM with Sway desktop |
| iso.nix | **Functional** | Installer ISO config |
| server.nix | **Functional** | Headless server config |

---

## Known Limitations

1. **No real transaction building**: `wallet/send` signs an intent string, not an RLP-encoded ETH transaction or a Solana transaction. Broadcasting requires external tooling.

2. **No network from keyd**: By design. keyd has `PrivateNetwork=true`. Signed transactions must be broadcast by the caller.

3. **Memory system is M0**: ZVEC vector search is not wired. Memory recall returns empty. Only ledger-based storage works.

4. **SafeSwitch doesn't execute the change**: `switch/begin` records the session but the caller must apply the NixOS change. The daemon manages the health-check/rollback lifecycle after the change.

5. **No end-to-end integration tests**: Each crate has unit tests. No tests verify the full daemon-to-daemon-to-bridge pipeline.

6. **Socket auth is file-permissions only**: No token-based auth for Unix socket access. Relies on filesystem permissions (0600/0660).

---

## Test Coverage

```
cargo test --workspace
```

| Crate | Tests | Status |
|-------|-------|--------|
| agentd | 11 | Pass (agent card, incidents, backup, hash chain, input validation, graceful shutdown) |
| osmoda-keyd | 21 | Pass (sign/verify, keccak256, encryption, KDF, decimal policy, delete) |
| osmoda-watch | 18 | Pass (state machine, persistence, health check serialization, input validation) |
| osmoda-routines | 17 | Pass (cron parser, persistence, scheduler, action execution) |
| osmoda-voice | 4 | Pass (STT missing binary, TTS missing binary, VAD record_clip, VAD record_segment) |
| agentctl | 0 | No tests |
| osmoda-egress | 0 | No tests |
| **Total** | **71** | **All pass** |

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
6. **NixOS-native deploy** — replace imperative nohup/iptables with osmoda.nix module on Hetzner
7. **Telegram bot** — `/start <order_id>` for server management via Telegram
8. **Email notifications** — send order_id + manage link after spawn
9. **Plan upgrades** — resize Hetzner server via management dashboard
