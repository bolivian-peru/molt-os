# Messaging Channels

Talk to your server from your phone. Telegram or WhatsApp.

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

### Step 2: Get your Telegram user ID

You need your numeric user ID to restrict access (so only you can control the bot).

1. Open Telegram
2. Search for **@userinfobot** and start a chat
3. Send any message — it replies with your user ID (a number like `123456789`)
4. Copy the number

### Step 3: Configure the server

SSH into your server and run:

```bash
# Save the bot token
echo 'YOUR_BOT_TOKEN' > /var/lib/osmoda/secrets/telegram-bot-token
chmod 600 /var/lib/osmoda/secrets/telegram-bot-token
```

Then add the Telegram channel to the gateway config:

```bash
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
```

Replace `YOUR_TELEGRAM_USER_ID` with the number from Step 2.

Restart the gateway:

```bash
systemctl restart osmoda-gateway
```

### Step 4: Test it

1. Open Telegram
2. Find your bot (search for the username you chose)
3. Send: "How's my server doing?"
4. Your server responds with a health check

That's it. Your server is now in your pocket.

### Security note

The `dmPolicy: 'allowlist'` + `allowFrom` setting means **only your Telegram account** can talk to the bot. Without this, anyone who discovers the bot can control your server. Always set it.

To add more users, add their numeric IDs to the `allowFrom` array:

```json
"allowFrom": ["123456789", "987654321"]
```

---

## WhatsApp Setup

1. Open the osModa web chat
2. Say: **"Set up WhatsApp"**
3. The AI enables the channel and shows you a QR code from the gateway logs
4. Scan the QR with WhatsApp (Settings > Linked Devices)
5. Send a message to the linked number

WhatsApp uses device-pairing (like WhatsApp Web), so no bot token is needed.

---

## How channels work

```
Phone (Telegram / WhatsApp)
  │
  ▼
OpenClaw Gateway
  │
  ├── Web UI         → osmoda agent  (Claude Opus, full detail)
  ├── Telegram       → mobile agent  (Claude Sonnet, concise)
  └── WhatsApp       → mobile agent  (Claude Sonnet, concise)
  │
  ▼
osmoda-bridge → agentd / keyd / watch / routines / mesh / ...
  │
  ▼
Audit ledger (every message logged with channel source)
```

- **Web chat** uses the `osmoda` agent (Claude Opus) — detailed, thorough responses
- **Telegram/WhatsApp** route to the `mobile` agent (Claude Sonnet) — concise, phone-friendly responses
- Both agents have the same 89 tools and full system access
- All channels share one audit trail

---

## NixOS module (declarative config)

If you prefer NixOS config over manual setup:

```nix
# configuration.nix
services.osmoda.channels.telegram = {
  enable = true;
  botTokenFile = "/var/lib/osmoda/secrets/telegram-bot-token";
  allowedUsers = [ "yourusername" ];
};

services.osmoda.channels.whatsapp = {
  enable = true;
  allowedNumbers = [ "+1234567890" ];
};
```

```bash
sudo nixos-rebuild switch
```

---

## Troubleshooting

**Bot doesn't respond?**
```bash
# Check gateway logs for channel connection
journalctl -u osmoda-gateway --since '5 min ago' | grep -i telegram
```

**"Unauthorized" or bot ignores messages?**
- Verify your user ID is in the `allowFrom` list
- Check: `cat /root/.openclaw/openclaw.json | jq '.channels.telegram'`

**Wrong bot token?**
```bash
# Overwrite with the correct one
echo 'CORRECT_TOKEN' > /var/lib/osmoda/secrets/telegram-bot-token
chmod 600 /var/lib/osmoda/secrets/telegram-bot-token
systemctl restart osmoda-gateway
```

**WhatsApp QR expired?**
```bash
systemctl restart osmoda-gateway
journalctl -u osmoda-gateway -f  # watch for new QR code
```
