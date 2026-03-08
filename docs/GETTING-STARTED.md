# Getting Started with osModa

Your first 10 minutes with an AI-managed server.

## Prerequisites

- A fresh VPS (Ubuntu 22.04/24.04 or Debian 12) — Hetzner, DigitalOcean, etc.
- An Anthropic API key or OAuth token (see [Auth](AUTH.md))
- SSH access to the server

---

## Step 1: Install

SSH into your server and run:

```bash
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | bash
```

This will:
1. Convert Ubuntu/Debian to NixOS (server reboots — SSH back in after ~3 min)
2. Build all 9 osModa daemons from source (~5 min on first build)
3. Install the OpenClaw AI gateway
4. Start all daemons

> **Warning:** This replaces your OS with NixOS. Use on fresh/disposable servers only.

### With API key at install time (recommended)

If you have your key ready, pass it during install — the gateway starts immediately:

```bash
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh \
  | bash -s -- --api-key sk-ant-api03-YOUR-KEY
```

### Verify installation

After install, all 9 daemons should be running:

```bash
curl -s --unix-socket /run/osmoda/agentd.sock http://localhost/health | jq
```

---

## Step 2: Set your API key (if you didn't pass it during install)

If you installed without `--api-key`, the gateway is enabled but not running yet. Set it up:

```bash
# 1. Write the environment file
echo 'ANTHROPIC_API_KEY=sk-ant-api03-YOUR_KEY' > /var/lib/osmoda/config/env
chmod 600 /var/lib/osmoda/config/env

# 2. Write auth profiles for both agents
KEY=sk-ant-api03-YOUR_KEY
for agent in osmoda mobile; do
  printf '{"type":"api_key","provider":"anthropic","key":"%s"}' "$KEY" \
    > /root/.openclaw/agents/$agent/agent/auth-profiles.json
done

# 3. Start the gateway
systemctl start osmoda-gateway
```

**Using an OAuth token** (`sk-ant-oat01-...`) instead? Change the auth profile format:

```bash
KEY=sk-ant-oat01-YOUR_TOKEN
for agent in osmoda mobile; do
  printf '{"type":"token","provider":"anthropic","token":"%s"}' "$KEY" \
    > /root/.openclaw/agents/$agent/agent/auth-profiles.json
done
```

See [Auth](AUTH.md) for details on API keys vs OAuth tokens.

---

## Step 3: Access the web chat

The gateway runs on `localhost:18789`. It's not exposed to the internet — you access it through an SSH tunnel.

### Open the tunnel

From your local machine (keep this terminal open):

```bash
ssh -N -L 18789:localhost:18789 root@YOUR-SERVER-IP
```

### Open in browser

Your gateway token was generated during install. Get it:

```bash
# Run on the server
cat /var/lib/osmoda/config/gateway-token
```

Then open:

```
http://localhost:18789?token=YOUR_GATEWAY_TOKEN
```

You're now talking to your server. The AI has full system access — 89 tools, 9 daemons, root-level control.

---

## Step 4: Talk to your server

Try these:

| You say | What happens |
|---------|-------------|
| "How's my server doing?" | Runs `system_health` + `system_discover`, shows CPU/RAM/disk/services |
| "What's using the most CPU?" | Queries processes sorted by CPU usage |
| "Show me nginx logs from the last hour" | Reads journal logs filtered by unit |
| "Set up a watcher for nginx" | Creates an autopilot health check with auto-restart |
| "What can you do?" | Lists capabilities based on what's actually running |

Every query is logged to the hash-chained audit ledger:

```bash
agentctl verify-ledger --state-dir /var/lib/osmoda
```

---

## Step 5: Connect Telegram (optional)

Manage your server from your phone. See [Channels](CHANNELS.md) for the full guide.

Quick version:

1. Open Telegram, search `@BotFather`, send `/newbot`
2. Pick a name, copy the bot token
3. On your server:

```bash
# Save the token
echo 'YOUR_BOT_TOKEN' > /var/lib/osmoda/secrets/telegram-bot-token
chmod 600 /var/lib/osmoda/secrets/telegram-bot-token

# Get your Telegram user ID (message @userinfobot on Telegram)
# Then update the gateway config:
node -e "
  var fs = require('fs');
  var config = JSON.parse(fs.readFileSync('/root/.openclaw/openclaw.json', 'utf8'));
  config.channels = config.channels || {};
  config.channels.telegram = {
    enabled: true,
    tokenFile: '/var/lib/osmoda/secrets/telegram-bot-token',
    dmPolicy: 'allowlist',
    allowFrom: ['YOUR_TELEGRAM_USER_ID']
  };
  fs.writeFileSync('/root/.openclaw/openclaw.json', JSON.stringify(config, null, 2));
"

# Restart gateway
systemctl restart osmoda-gateway
```

4. Find your bot on Telegram, send it a message. Your server responds.

---

## What's next

- **Deploy an app** — "Deploy my Node.js API on port 3000"
- **Set up monitoring** — "Watch nginx and restart it if it goes down"
- **Connect another server** — "Create a mesh invite"
- **Check the audit trail** — "Show me everything that changed today"
- **Security hardening** — "Run a security audit"

---

## Useful commands

```bash
# System health
curl -s --unix-socket /run/osmoda/agentd.sock http://localhost/health | jq

# Check all daemons
for svc in osmoda-agentd osmoda-keyd osmoda-watch osmoda-routines osmoda-mesh osmoda-mcpd osmoda-teachd osmoda-voice osmoda-egress; do
  printf '%-20s %s\n' $svc $(systemctl is-active $svc)
done

# Gateway logs
journalctl -u osmoda-gateway -f

# Verify audit ledger integrity
agentctl verify-ledger --state-dir /var/lib/osmoda

# View recent events
agentctl events --state-dir /var/lib/osmoda --limit 20

# Emergency rollback (bypasses AI)
sudo nixos-rebuild --rollback switch
```

---

## Troubleshooting

**Gateway won't start?**
```bash
journalctl -u osmoda-gateway --since '5 min ago'
# Common: missing auth-profiles.json or empty API key
```

**"Connection refused" on localhost:18789?**
- Make sure your SSH tunnel is running (`ssh -N -L 18789:localhost:18789 ...`)
- Check gateway is active: `systemctl is-active osmoda-gateway`

**Daemons not starting?**
```bash
systemctl status osmoda-agentd
journalctl -u osmoda-agentd --since '5 min ago'
```

**Need to start over?** NixOS rollback is always available:
```bash
sudo nixos-rebuild --rollback switch
```
