# Getting Started with osModa

*Last updated: 2026-04-18. Reflects the v1.2 modular runtime.*

Your first 10 minutes with an AI-managed server.

## Prerequisites

- A fresh VPS (Ubuntu 22.04/24.04 or Debian 12) — Hetzner, DigitalOcean, etc.
- One of:
  - A **Claude Pro / Max OAuth token** (`sk-ant-oat01-…`) — cheapest for heavy agent use
  - An **Anthropic Console API key** (`sk-ant-api03-…`) — pay-per-token
  - Or you can defer this and add credentials after install
- SSH access to the server

See [AUTH.md](AUTH.md) for a breakdown of the trade-offs.

---

## Step 1: Install

SSH into your server and run the installer. The simplest form takes no flags:

```bash
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash
```

This:

1. Converts Ubuntu/Debian to NixOS (server reboots — SSH back in after ~3 min)
2. Builds all 10 Rust daemons from source (~5 min on first build)
3. Installs `osmoda-gateway` (TypeScript, always the systemd unit) + the Claude Code CLI
4. Starts every daemon

> **Warning:** This replaces your OS with NixOS. Use on fresh/disposable servers only.

### Pre-configure credentials at install time (recommended)

You can pass everything on the install command. The agent is ready the moment the gateway boots:

```bash
# Claude Pro OAuth — cheapest for heavy use
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash -s -- \
  --default-model claude-opus-4-6 \
  --credential "My Claude Pro|anthropic|oauth|$(printf 'sk-ant-oat01-…' | base64)"

# Or: Console API key (pay-per-token)
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash -s -- \
  --default-model claude-opus-4-6 \
  --credential "Anthropic Console|anthropic|api_key|$(printf 'sk-ant-api03-…' | base64)"

# Or the legacy one-liner (auto-promotes to a credential):
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash -s -- \
  --api-key sk-ant-api03-YOUR-KEY
```

Available flags:

| Flag | Values | Notes |
|---|---|---|
| `--credential` | `label\|provider\|type\|base64-secret` | Repeatable. `provider` ∈ {`anthropic`, `openai`, `openrouter`}. `type` ∈ {`oauth`, `api_key`}. |
| `--default-model` | e.g. `claude-opus-4-6`, `claude-sonnet-4-6` | Seeds the `osmoda` agent's default model. |
| `--runtime` | `claude-code` (default) or `openclaw` | Picks the initial per-agent runtime. Changeable later without re-running install. |
| `--api-key` | raw or base64 | Legacy single-credential shortcut. |

### Verify

After the reboot, every daemon should be active:

```bash
for svc in osmoda-agentd osmoda-keyd osmoda-watch osmoda-routines \
           osmoda-mesh osmoda-mcpd osmoda-teachd osmoda-voice \
           osmoda-egress osmoda-gateway; do
  printf '%-22s %s\n' "$svc" "$(systemctl is-active "$svc")"
done
curl -s --unix-socket /run/osmoda/agentd.sock http://localhost/health | jq
```

---

## Step 2: Add a credential (if you didn't pass one at install)

Three ways to do this — all produce the same result (an encrypted record in `/var/lib/osmoda/config/credentials.json.enc`).

### Option A: From the dashboard

Open the web UI (see Step 3 for how to access it) → **Engine** tab → **Credentials** → **+ Add credential** → paste your `sk-ant-oat01-…` or `sk-ant-api03-…` → **Add + test**.

### Option B: From the REST API

```bash
TOKEN=$(cat /var/lib/osmoda/config/gateway-token)
curl -X POST http://127.0.0.1:18789/config/credentials \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{
    "label":    "My Claude Pro",
    "provider": "anthropic",
    "type":     "oauth",
    "secret":   "sk-ant-oat01-…"
  }'
# → { "credential": { "id": "cred_…", "secret_preview": "sk-ant-oat0…abcd" } }

# Then assign it to the osmoda agent:
curl -X PATCH http://127.0.0.1:18789/config/agents/osmoda \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"credential_id":"cred_…","model":"claude-opus-4-6"}'

# Test it:
curl -X POST http://127.0.0.1:18789/config/credentials/cred_…/test \
  -H "Authorization: Bearer $TOKEN"
# → { "ok": true, "model_list": ["claude-opus-4-6", …] }
```

### Option C: CLI one-liner

```bash
# Absorbs legacy api-key files on next gateway boot
echo 'sk-ant-api03-…' > /var/lib/osmoda/config/api-key
chmod 600 /var/lib/osmoda/config/api-key
systemctl restart osmoda-gateway
```

---

## Step 3: Access the web chat

The gateway binds to `127.0.0.1:18789` — it's not exposed to the internet. You reach it through an SSH tunnel.

```bash
# On your local machine (keep this terminal open):
ssh -N -L 18789:127.0.0.1:18789 root@YOUR-SERVER-IP
```

Get the gateway token:

```bash
# On the server
cat /var/lib/osmoda/config/gateway-token
```

Open in your browser:

```
http://localhost:18789?token=YOUR_GATEWAY_TOKEN
```

You're now talking to your server. The agent has full system access — 91 MCP tools across 10 daemons, with root-level control mediated by structured tool calls (not raw shell).

---

## Step 4: Talk to your server

Try these:

| You say | What happens |
|---|---|
| "How's my server doing?" | Runs `system_health` + `system_discover`, shows CPU/RAM/disk/services |
| "What's using the most CPU?" | Queries processes sorted by CPU usage |
| "Show me nginx logs from the last hour" | Reads journal logs filtered by unit |
| "Set up a watcher for nginx" | Creates an autopilot health check with auto-restart |
| "What can you do?" | Lists capabilities based on what's actually running |

Every query routes through `agentd` and logs to the hash-chained audit ledger:

```bash
agentctl verify-ledger --state-dir /var/lib/osmoda
agentctl events --state-dir /var/lib/osmoda --limit 20
```

---

## Step 5: Switch the agent engine (optional)

osModa ships with two drivers — `claude-code` (Anthropic's official CLI, supports OAuth + API key) and `openclaw` (legacy, API key only). Default is `claude-code`. You can swap at runtime, no SSH or rebuild:

- **Dashboard:** Engine tab → Agents section → `osmoda` card → Runtime dropdown → pick `OpenClaw (legacy)` → Save.
- **REST API:**
  ```bash
  curl -X PATCH http://127.0.0.1:18789/config/agents/osmoda \
    -H "Authorization: Bearer $(cat /var/lib/osmoda/config/gateway-token)" \
    -H "Content-Type: application/json" \
    -d '{"runtime":"openclaw"}'
  ```

Save fires `SIGHUP`; in-flight sessions keep running on their original driver; the next message uses the new one. Zero downtime.

OpenClaw only accepts API-key credentials (Anthropic disabled OAuth for it). If your current credential is OAuth-only, add a second credential with `type: api_key` first.

---

## Step 6: Connect Telegram (optional)

Manage your server from your phone. See [CHANNELS.md](CHANNELS.md) for the full guide.

Quick version:

1. Open Telegram, search `@BotFather`, send `/newbot`. Pick a name, copy the bot token.
2. Get your Telegram user ID — message `@userinfobot` on Telegram.
3. On the server:
   ```bash
   echo 'YOUR_BOT_TOKEN' > /var/lib/osmoda/secrets/telegram-bot-token
   chmod 600 /var/lib/osmoda/secrets/telegram-bot-token

   # Add the allowed user to /etc/nixos/configuration.nix:
   #   services.osmoda.gateway.telegram = {
   #     enable = true;
   #     tokenFile = "/var/lib/osmoda/secrets/telegram-bot-token";
   #     allowedUsers = [ "YOUR_TELEGRAM_USERNAME" ];
   #   };
   nixos-rebuild switch
   ```
4. Message your bot on Telegram. The `mobile` agent responds with Sonnet (concise, phone-optimized).

---

## What's next

- **Deploy an app** — "Deploy my Node.js API on port 3000"
- **Set up monitoring** — "Watch nginx and restart it if it goes down"
- **Connect another server** — "Create a mesh invite" ([mesh p2p](ARCHITECTURE.md#agent-gateway--modular-runtime-v02))
- **Check the audit trail** — "Show me everything that changed today"
- **Security hardening** — "Run a security audit"
- **Read the security model** — [SECURITY.md](SECURITY.md) explains the four trust boundaries and what they protect

---

## Useful commands

```bash
# System health
curl -s --unix-socket /run/osmoda/agentd.sock http://localhost/health | jq

# All daemons at once
for svc in osmoda-agentd osmoda-keyd osmoda-watch osmoda-routines \
           osmoda-mesh osmoda-mcpd osmoda-teachd osmoda-voice \
           osmoda-egress osmoda-gateway; do
  printf '%-22s %s\n' "$svc" "$(systemctl is-active "$svc")"
done

# Gateway live logs
journalctl -u osmoda-gateway -f

# Reload agents.json without dropping WS sessions
systemctl reload osmoda-gateway

# Verify audit ledger integrity
agentctl verify-ledger --state-dir /var/lib/osmoda

# Recent events
agentctl events --state-dir /var/lib/osmoda --limit 20

# Emergency rollback (bypasses agent)
sudo nixos-rebuild --rollback switch
```

---

## Troubleshooting

**Gateway won't start?**
```bash
journalctl -u osmoda-gateway --since '5 min ago'
# Common: agents.json missing credential_id, or credential was removed
```

**`Agent <id> has no credential configured`**
Open the dashboard → Engine tab → Agents → re-select a credential, save. Or via REST as shown in Step 2.

**`Credential cred_… not found`**
The credential was deleted but an agent still references it. Same fix — reassign.

**Credential test returns `HTTP 401`**
The secret is wrong, revoked upstream, or truncated on paste. Verify in the provider's dashboard (Anthropic Console, OpenAI) and re-add.

**Need to start over?** NixOS rollback:
```bash
sudo nixos-rebuild --rollback switch
```
