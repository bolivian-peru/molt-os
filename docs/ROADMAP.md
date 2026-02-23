# osModa — Feature Roadmap

Last updated: 2026-02-23

Current state: 97 tests passing, 7 Rust daemons + 1 CLI, 45 bridge tools, 15 skills.
This document covers what's being added next, in priority order.

---

## What's Live Today

| Feature | Where |
|---------|-------|
| Full system access (processes, files, services, kernel) | agentd + bridge |
| Crypto wallets — ETH + SOL, AES-256-GCM, policy-gated | osmoda-keyd |
| SafeSwitch deploys with auto-rollback | osmoda-watch |
| Background automation (cron, interval, event) | osmoda-routines |
| Voice — 100% local STT + TTS, no cloud | osmoda-voice |
| Hash-chained audit ledger | agentd ledger |
| Telegram + WhatsApp channel config | osmoda.nix |
| Web chat UI | osmoda-ui |
| One-command server provisioning with USDC payments | spawn.os.moda |
| **P2P encrypted agent-to-agent mesh** | **osmoda-mesh** |

### osmoda-mesh — what it does

Any two osModa instances can form a direct encrypted connection. Invite code → peer accepts → Noise_XX handshake + ML-KEM-768 hybrid post-quantum key exchange → encrypted TCP channel. No central server. No global registry. Post-quantum by default.

```
osModa server A                    osModa server B
  osmoda-mesh                        osmoda-mesh
       │                                  │
       │  1. A creates invite code        │
       │  2. B accepts invite             │
       │  3. Noise_XX handshake ──────────│
       │  4. ML-KEM-768 PQ exchange ──────│
       │  5. Hybrid re-key ───────────────│
       │  6. Encrypted messages ──────────│
```

**Mesh bridge tools** (7): `mesh_identity`, `mesh_invite_create`, `mesh_invite_accept`, `mesh_peers`, `mesh_peer_send`, `mesh_peer_disconnect`, `mesh_health`

**Cipher suite**: Noise_XX (X25519/ChaChaPoly/BLAKE2s) + ML-KEM-768 (FIPS 203) + HKDF-SHA256 hybrid. If classical crypto breaks, PQ protects. If PQ breaks, classical protects.

**Port**: 18800 TCP. **Socket**: `/run/osmoda/mesh.sock`.

---

## Sprint 1 — Remote Access + Safety (This Week)

The biggest gap right now: osModa installed on a home server is only accessible via SSH tunnel. These fix that.

### 1. Cloudflare Tunnel Integration

**What it does:** Gives your osModa server a persistent public URL with zero port forwarding, zero firewall config. Works from any device, any network.

**How:** cloudflared daemon opens an outbound connection to Cloudflare's edge. They route traffic back to localhost. Free tier is unlimited for personal use.

**NixOS config:**
```nix
services.osmoda.remoteAccess.cloudflare = {
  enable = true;
  credentialFile = "/var/lib/osmoda/secrets/cloudflare-tunnel";
  hostname = "osmoda.yourdomain.com";  # optional — leave blank for free trycloudflare.com URL
};
```

**Setup flow:**
1. Enable in NixOS config
2. Run `cloudflared tunnel login` once — saves credentials
3. `nixos-rebuild switch`
4. Access osModa from anywhere via the assigned URL

**Files:** `nix/modules/osmoda.nix` (~25 LOC), `docs/REMOTE-ACCESS.md`

**Latency:** 50-100ms added (Cloudflare edge routing). Fine for chat.

---

### 2. Tailscale Integration

**What it does:** Creates a WireGuard mesh VPN between your devices. Install Tailscale on your phone → access osModa directly via Tailscale IP. Zero config, end-to-end encrypted, no third party in the data path.

**NixOS config:**
```nix
services.osmoda.remoteAccess.tailscale = {
  enable = true;
  # Run once: sudo tailscale up --auth-key=tskey-...
};
```

**Best for:** Personal use, team access, situations where you don't want Cloudflare handling your traffic.

**Free tier:** 100 devices, unlimited bandwidth.

**Files:** `nix/modules/osmoda.nix` (~15 LOC)

**Latency:** 5-20ms (direct WireGuard, hole-punched). Faster than Cloudflare Tunnel.

---

### 3. Safety Slash Commands

**What it does:** Commands that bypass the AI entirely. When the AI is broken, stuck, or you need instant action — these work.

**Commands:**
```
/rollback       — immediately nixos-rebuild --rollback, no AI involved
/status         — raw health dump direct from agentd, bypasses conversation
/panic          — stop all services, rollback NixOS, log to ledger
/restart        — restart osmoda-gateway (use when AI is unresponsive)
```

**Why this matters:** RosClaw ships `/estop` that kills the robot regardless of what the AI is doing. osModa needs the equivalent for servers. If OpenClaw gets into a loop or the AI does something wrong, you need an escape hatch that doesn't go through the AI.

**Implementation:** OpenClaw slash commands registered at plugin level, execute shell commands directly, skip the AI pipeline entirely.

**Files:** `packages/osmoda-bridge/index.ts` (~80 LOC)

---

### 4. Discord Channel Config

**What it does:** Adds Discord as a messaging channel alongside Telegram + WhatsApp.

Discord is the natural habitat for developers — larger audience than either other channel for a dev tool.

**NixOS config:**
```nix
services.osmoda.channels.discord = {
  enable = true;
  botTokenFile = "/var/lib/osmoda/secrets/discord-bot-token";
  allowedGuildIds = [ "123456789" ];  # restrict to your server
};
```

**Setup via chat (same as Telegram):**
> "Connect Discord"
> AI guides through: create bot at discord.com/developers → copy token → paste it

**Files:** `nix/modules/osmoda.nix` (~20 LOC), `templates/AGENTS.md` (+Discord setup instructions)

---

## Sprint 2 — Intelligence + Reliability (Next Week)

### 5. Service Discovery Tool

**What it does:** The AI asks "what's running on this server?" and gets a real answer — not a hardcoded list of things we thought might be there. Discovers running services, listening ports, API endpoints.

**Tool:** `system_discover`

```json
{
  "found": [
    { "name": "nginx", "port": 80, "pid": 1234, "health_url": "http://localhost/health" },
    { "name": "postgres", "port": 5432, "pid": 5678 },
    { "name": "redis", "port": 6379, "pid": 9012 },
    { "name": "my-api", "port": 3000, "pid": 3456, "detected_as": "node" }
  ]
}
```

**Why this matters:** Right now osModa knows about the services we hardcoded into AGENTS.md. If you install nginx after setup, the AI doesn't "see" it unless you tell it. With service discovery, the AI genuinely knows what's on your machine — scanning `ss -tlnp`, reading `/etc/systemd/system/`, checking `/proc`.

Inspired by RosClaw's `ros2-introspect` tool — the robot AI dynamically discovers what capabilities the robot has. Same principle: OS AI dynamically discovers what's running.

**Files:** `crates/agentd/src/api/` (new `discovery.rs`, ~150 LOC), `packages/osmoda-bridge/index.ts` (~30 LOC)

---

### 6. FTS5 Memory Recall

**What it does:** Makes `memory_recall` actually return results. Currently returns empty — ZVEC vector search is unimplemented. Replace with SQLite FTS5 keyword search over the ledger.

**Reality check:** Full vector search (ZVEC, nomic embeddings) is correct long-term. But the infrastructure adds 500MB+ and significant complexity. FTS5 is built into SQLite, zero dependencies, handles keyword search well for event recall.

**What changes:**
- Add FTS5 virtual table to `ledger.rs`: indexed on `type + actor + payload`
- Implement `memory_recall` as BM25-ranked FTS5 query
- Bridge `memory_recall` tool returns real results

**Later:** When ZVEC is ready, FTS5 becomes the hybrid complement (BM25 + vector via RRF), not a replacement.

**Files:** `crates/agentd/src/ledger.rs` (~40 LOC), `crates/agentd/src/api/memory.rs` (~50 LOC)

---

### 7. Zod Config Validation in Bridge

**What it does:** Validates all bridge configuration (socket paths, timeouts, env vars) at startup with clear error messages. Learned from RosClaw — they validate plugin config via Zod schema before the plugin loads.

Currently osModa reads `process.env` raw — a missing socket path silently fails at first use.

```typescript
// Before: silent failure at runtime
const AGENTD_SOCKET = process.env.OSMODA_SOCKET || "/run/osmoda/agentd.sock";

// After: fail fast with clear message at startup
const config = BridgeConfigSchema.parse({
  agentdSocket: process.env.OSMODA_SOCKET ?? "/run/osmoda/agentd.sock",
  keydSocket: process.env.OSMODA_KEYD_SOCKET ?? "/run/osmoda/keyd.sock",
  // ...
});
// If wrong: "BridgeConfig invalid: agentdSocket must be an absolute path"
```

**Files:** `packages/osmoda-bridge/index.ts` (~40 LOC)

---

## Sprint 3 — Product Polish (Following Week)

### 8. Mobile-Friendly Status Dashboard

**What it does:** `GET /status` on agentd returns a self-contained HTML page — server-rendered, no JavaScript required, works in any mobile browser via SSH tunnel or Cloudflare URL.

Shows: hostname, uptime, CPU/RAM/disk gauges, service status grid (green/red), last 5 ledger events.

Auto-refresh every 30s via `<meta http-equiv="refresh">`. Loads in <10KB.

**Use case:** Open your phone, check if the server is healthy in 2 seconds. No chat session needed.

**Files:** `crates/agentd/src/api/status_page.rs` (~200 LOC)

---

### 9. Daily Briefing via Telegram

**What it does:** Every morning at 07:00, the server sends you a summary:

```
📊 osmoda-dev — Mon 24 Feb

Uptime: 12d 4h
CPU: 3% avg (24h)
Disk: 42% used, 58GB free
RAM: 2.1 / 8.0 GB

Events (24h): 127 total, 0 errors
Services: all healthy ✓

Last incident: none
```

Sends via Telegram Bot API. No AI involved — direct from osmoda-routines.

**Config:**
```nix
services.osmoda.channels.telegram.dailyBriefing = {
  enable = true;
  time = "07:00";
  chatId = "123456789";  # your Telegram chat ID
};
```

Or tell the AI: "Send me a daily morning briefing via Telegram at 7am"

**Files:** `crates/osmoda-routines/src/routine.rs` (~100 LOC), `nix/modules/osmoda.nix` (~15 LOC)

---

## Sprint 4 — Mesh-Powered Features (Month 2)

osmoda-mesh is implemented. These features build on top of it.

### 10. Multi-Server Management via Mesh

**What it does:** One osModa instance manages multiple servers. Mesh peers report health, the AI aggregates it.

```
You: "Check all my servers"

osModa:
  web-01 (Berlin)    — healthy, 12% CPU
  db-01 (Frankfurt)  — healthy, 34% CPU
  worker-01 (NYC)    — ⚠ high memory (89%)

  worker-01 needs attention. Want me to investigate?
```

**How it works with mesh:**
- Each server already has osmoda-mesh running (port 18800)
- Peer to peer via `mesh_invite_create` / `mesh_invite_accept` on initial setup
- Hub server calls `mesh_peer_send` with `health_report` message type
- Peers respond with live CPU/RAM/disk/service data
- The AI aggregates responses and presents the combined view

**Relationship to spawn.os.moda:** All provisioned servers already appear in spawn's database. The management dashboard already shows them. The next step: mesh connects them so the AI can query all at once.

**Files:**
- `crates/osmoda-mesh/src/messages.rs` — extend `HealthReport` message with service states
- `packages/osmoda-bridge/index.ts` — `mesh_fleet_status` tool (~50 LOC)
- `templates/AGENTS.md` — multi-server fleet management instructions

---

### 11. Spawn Peer Discovery via Mesh

**What it does:** When you spawn multiple servers, they automatically pair via mesh. No manual invite exchange.

**How:**
1. spawn.os.moda includes each server's mesh public key + endpoint in the cloud-init
2. Each server phone-homes to spawn with its mesh public identity on first boot
3. spawn's `/api/peers?order_id=X` returns peer endpoints for servers in the same account
4. Each server fetches its peer list and initiates mesh connections

**Result:** Buy 3 servers, they connect to each other automatically. AI can query all 3 without manual setup.

**Files:**
- `apps/spawn/server.js` — `/api/peers` endpoint + store mesh identity in heartbeat (~80 LOC)
- `nix/modules/osmoda.nix` — on-boot peer fetch script (~30 LOC)
- `crates/osmoda-mesh/src/` — auto-connect on startup if peers file exists

---

### 12. A2UI Live Dashboard (Canvas)

**What it does:** Inspired by RosClaw's `openclaw-canvas`. The AI sends structured data, the client renders it as interactive widgets — not just text.

**RosClaw uses A2UI (Agent-to-UI, Google Apache 2.0):** Declarative JSONL protocol. AI sends component trees. Client renders them. Same AI, different outputs depending on client type:
- Telegram/WhatsApp → text summary
- Web browser → live gauges, tables, charts
- Mobile app → native components

**For osModa this means:**
```
User: "How's the server doing?"

[Telegram] osModa: Uptime 12d, CPU 3%, RAM 26%, all services healthy.

[Web dashboard] osModa:
  ┌─────────────────────────────────────┐
  │ CPU  ████░░░░░░ 38%                 │
  │ RAM  ██░░░░░░░░ 26%  (2.1/8 GB)    │
  │ Disk █████░░░░░ 42%  (42/100 GB)   │
  ├─────────────────────────────────────┤
  │ nginx     ● running  (42d 3h)       │
  │ postgres  ● running  (42d 3h)       │
  │ redis     ● running  (12d 1h)       │
  └─────────────────────────────────────┘
```

**Implementation path:**
1. Bridge tool `canvas_present` sends A2UI JSONL to OpenClaw
2. osmoda-ui interprets A2UI messages alongside regular chat
3. Renders component tree as live HTML widgets
4. Auto-updates when AI sends `dataModelUpdate`

**Files:** `packages/osmoda-bridge/index.ts` (canvas tools, ~100 LOC), `packages/osmoda-ui/index.html` (A2UI renderer, ~300 LOC)

---

### 13. WebRTC P2P via spawn.os.moda

**What it does:** Connect to your home server from a browser — no Cloudflare, no VPN client, no port forwarding. Direct encrypted P2P from browser to server.

**Why this is different from mesh:**
- osmoda-mesh: server-to-server, Noise_XX, always-on P2P between osModa daemons
- WebRTC: browser-to-server, DTLS/SRTP, for the web UI access case

**Architecture:**
```
Your browser
    ↓ 1. Fetch session offer
spawn.os.moda (signaling server — already running)
    ↓ 2. Exchange SDP + ICE candidates
Your osModa server (already phoning home to spawn)
    ↓ 3. WebRTC hole-punch (85-92% direct P2P)
    OR fallback to TURN relay (8-15%)
    ↓ 4. Encrypted data channel
osModa web UI (same as SSH tunnel, but browser-native)
```

**Why spawn.os.moda is the natural signaling server:**
- Every osModa server already has a relationship with spawn.os.moda (heartbeat, order_id)
- Home server registers its WebRTC offer on startup
- Browser fetches the offer, completes the handshake, connects

**Files:**
- `apps/spawn/server.js` (signaling endpoints, ~150 LOC)
- `packages/osmoda-bridge/` (client-side WebRTC, ~200 LOC)
- `nix/modules/osmoda.nix` (register with spawn on startup, ~20 LOC)

---

## Summary Table

| # | Feature | Sprint | Effort | Impact |
|---|---------|--------|--------|--------|
| — | **osmoda-mesh P2P** | **DONE** | **3 days** | **Encrypted server-to-server, PQ-safe** |
| 1 | Cloudflare Tunnel integration | 1 | 0.5 day | Home server users can access from anywhere |
| 2 | Tailscale integration | 1 | 0.5 day | Team/VPN mesh access |
| 3 | Safety slash commands | 1 | 1 day | Production safety net |
| 4 | Discord channel | 1 | 0.5 day | Developer audience |
| 5 | Service discovery tool | 2 | 2 days | AI genuinely knows what's on the machine |
| 6 | FTS5 memory recall | 2 | 1 day | Memory actually works |
| 7 | Zod config validation | 2 | 0.5 day | Developer experience, fail-fast |
| 8 | Mobile status dashboard | 3 | 1 day | Quick phone health check |
| 9 | Daily Telegram briefing | 3 | 1 day | Proactive awareness |
| 10 | Multi-server management | 4 | 1 week | Fleet view, AI queries all servers at once |
| 11 | Spawn peer discovery | 4 | 3 days | Auto-connect provisioned servers via mesh |
| 12 | A2UI live dashboard | 4 | 1 week | Product differentiator, native apps |
| 13 | WebRTC via spawn.os.moda | 5 | 2 weeks | Browser-native access, no install |

---

## Design Principles for New Features

These come from RosClaw analysis + code-simplifier philosophy:

1. **Talk to set up, not edit config** — Every new feature should be configurable by telling the AI, not editing NixOS files. Config files are the fallback for advanced users.

2. **One conversation, all channels** — New channels join the existing conversation. The AI is one mind across web, Telegram, WhatsApp, Discord.

3. **No third party in the data path** — Where possible, keep data off cloud services. Cloudflare Tunnel is a pragmatic exception. osmoda-mesh (Noise_XX + ML-KEM-768) and WebRTC P2P are the principled long-term answers.

4. **Fail fast, fail loudly** — Config errors surface at startup, not at runtime. Zod validation, socket existence checks, required credential checks — all at boot.

5. **Safety bypasses AI** — `/rollback`, `/panic`, `/status` must work even when the AI is broken. Never route emergency commands through the AI pipeline.

6. **Narrow tools over wide tools** — A `service_discover` tool that returns structured data is better than `shell_exec ls /proc`. Structure enables AI reasoning; raw output requires more prompt tokens.

7. **Cryptography is not optional** — osmoda-mesh ships with post-quantum key exchange by default. Security degrades gracefully (if PQ breaks, classical protects), never silently.
