# Messaging Channels

*Last updated: 2026-04-18. Reflects the v1.2 modular runtime.*

Talk to your server from your phone. Telegram is live; WhatsApp is documented but the current MCP-based channel bridge for WA is minimal — expect rough edges there.

---

## How channels route to agents (v1.2)

```
Phone (Telegram / WhatsApp)
  │
  ▼
osmoda-gateway (one systemd unit, modular — reads agents.json)
  │
  ├── Web UI          → agent bound to channel "web"      (default: osmoda, Opus)
  ├── Telegram        → agent bound to channel "telegram" (default: mobile, Sonnet)
  └── WhatsApp        → agent bound to channel "whatsapp" (default: mobile, Sonnet)
  │
  ▼  per-session driver lookup
  ├── claude-code driver   → spawns `claude` CLI
  └── openclaw driver      → spawns `openclaw` binary
  │
  ▼
91 MCP tools over stdio → agentd / keyd / watch / routines / mesh / voice / mcpd / teachd
  │
  ▼
Audit ledger (every message + every tool call logged with channel source)
```

Channel-to-agent routing lives in `/var/lib/osmoda/config/agents.json`:

```json
{
  "bindings": [
    { "channel": "telegram", "agent_id": "mobile" },
    { "channel": "whatsapp", "agent_id": "mobile" }
  ]
}
```

Change the binding via the dashboard Engine tab, or with a PATCH:

```bash
TOKEN=$(cat /var/lib/osmoda/config/gateway-token)
curl -X PUT http://127.0.0.1:18789/config/agents \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{ "agents": [...], "bindings": [
    {"channel":"telegram", "agent_id":"my-custom-agent"},
    {"channel":"whatsapp", "agent_id":"mobile"}
  ]}'
```

Each agent independently picks its runtime (claude-code or openclaw), credential, and model — see [AUTH.md](AUTH.md) for how credentials work in v1.2.

---

## Telegram Setup

### What you need

- A Telegram account
- Your osModa server running with the gateway active
- 5 minutes

### Step 1: Create a Telegram bot

1. Open Telegram on your phone or desktop
2. Search for **@BotFather** and start a chat
3. Send `/newbot`
4. Pick a display name (e.g. "My Server")
5. Pick a username (must end in `bot`, e.g. `myserver_osmoda_bot`)
6. BotFather gives you a token like: `7123456789:AAF1x2y3z4-abcDEFghiJKLmnoPQRstu`
7. Copy the token

### Step 2: Get your Telegram username or user ID

You need this to restrict access. Either works:

- Your **username** (starting `@`) — easiest, but only if you set one
- Your **numeric user ID** — message `@userinfobot` on Telegram; it replies with the number

### Step 3: Configure the server

SSH into your server and save the token + set allowed users via NixOS config:

```bash
echo 'YOUR_BOT_TOKEN' > /var/lib/osmoda/secrets/telegram-bot-token
chmod 600 /var/lib/osmoda/secrets/telegram-bot-token
```

Edit `/etc/nixos/configuration.nix`:

```nix
services.osmoda.gateway.telegram = {
  enable = true;
  tokenFile = "/var/lib/osmoda/secrets/telegram-bot-token";
  allowedUsers = [ "YOUR_TELEGRAM_USERNAME" ];  # or ["123456789"] for numeric IDs
};
```

Rebuild:

```bash
nixos-rebuild switch
```

### Step 4: Test it

1. Open Telegram
2. Find your bot (search for the username you chose)
3. Send: "How's my server doing?"
4. Your server responds with a health check

That's it. Your server is now in your pocket.

### Security note

The `allowedUsers` list is an **allowlist**. If empty, the bot ignores every update. Without this, anyone who discovers the bot can control your server. Always set it.

To add more users, extend the list:

```nix
allowedUsers = [ "alice_username" "987654321" ];
```

Usernames and numeric IDs both work.

---

## WhatsApp Setup

WhatsApp integration today requires the `whatsapp-mcp` MCP server (or equivalent) — managed via `osmoda-mcpd`. The channel routes the same way as Telegram (bound to the `mobile` agent by default), but the bridge is less mature than Telegram and has fewer tests.

If you want to use it:

1. Install a WhatsApp MCP server via `mcp_servers` config (see [MCP-ECOSYSTEM.md](MCP-ECOSYSTEM.md))
2. Bind the `whatsapp` channel to your preferred agent in `agents.json`
3. Follow the MCP server's device-pairing flow (usually a QR scan in Telegram logs)

Treat this as beta. If it breaks, fall back to Telegram.

---

## Troubleshooting

**Bot doesn't respond?**
```bash
# Gateway logs for the Telegram dispatch
journalctl -u osmoda-gateway --since '5 min ago' | grep -iE 'telegram|webhook|channel'
```

**"Telegram agent's credential is missing."**
The agent bound to `telegram` has no `credential_id`, or the credential was deleted. Open the dashboard → Engine tab → Agents → re-select a credential on the mobile agent → Save.

**"Telegram agent not configured."**
No agent is bound to the `telegram` channel. Add a binding — see the channels routing section above.

**Unauthorized / bot ignores your messages**
Your username/ID isn't in `allowedUsers`. Check with:
```bash
systemctl show osmoda-gateway --property=Environment | grep TELEGRAM_ALLOWED_USERS
```

**Wrong bot token?**
```bash
echo 'CORRECT_TOKEN' > /var/lib/osmoda/secrets/telegram-bot-token
chmod 600 /var/lib/osmoda/secrets/telegram-bot-token
systemctl restart osmoda-gateway
```

**Agent replies are too long for a Telegram message**
The gateway chunks replies at 4000 chars to respect Telegram's 4096-char limit. If the agent's outputs are consistently truncated, switch that agent's model to Sonnet or Haiku (shorter responses by temperament) via the Engine tab.
