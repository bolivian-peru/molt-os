# Messaging Channels

Talk to your server from your phone. Telegram or WhatsApp.

---

## The easy way (just talk to it)

Open the osModa web chat and say:

> "Connect my Telegram"

or

> "Set up WhatsApp"

The AI walks you through it. It saves the credentials, configures the channel, restarts the gateway. You don't edit any files.

---

## Telegram — what happens

1. You tell osModa "connect Telegram"
2. It tells you to create a bot via @BotFather on Telegram
3. You paste the bot token into the chat
4. osModa saves the token, enables the channel, restarts the gateway
5. You open Telegram, find your bot, send a message
6. The bot IS your server. Same AI, same tools, same audit trail.

**Time:** 2 minutes.

## WhatsApp — what happens

1. You tell osModa "set up WhatsApp"
2. It enables the channel and restarts the gateway
3. It reads the gateway log and shows you a QR code
4. You scan the QR with WhatsApp on your phone (Settings > Linked Devices)
5. Send a message to the linked number. Your server responds.

**Time:** 1 minute.

---

## Chat sync

All channels share one conversation. There is one OS, one mind, one thread.

- Message from Telegram? Shows in web chat too.
- Reply from web chat? If you started the conversation from Telegram, the reply goes back to Telegram.
- Every message, every channel — logged to the audit ledger.

The AI adapts to the channel:
- **Web chat**: Full detail, code blocks, long explanations
- **Telegram/WhatsApp**: Shorter, punchier — you're on your phone

---

## Restricting access

When setting up channels, osModa will ask if you want to restrict who can talk to the bot.

- **Telegram**: Provide your username. Only you can use the bot.
- **WhatsApp**: Provide your phone number. Only you can message it.

Without restrictions, anyone who discovers the bot can control your server. Set them.

---

## The manual way (NixOS config)

If you prefer declarative config over chatting:

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
# Save token first
echo 'YOUR_BOT_TOKEN_FROM_BOTFATHER' > /var/lib/osmoda/secrets/telegram-bot-token
chmod 600 /var/lib/osmoda/secrets/telegram-bot-token

# Apply
sudo nixos-rebuild switch
```

The NixOS module generates an OpenClaw config file and passes it to the gateway.

---

## How it works

```
Phone (Telegram / WhatsApp)
  │
  ▼
OpenClaw Gateway (single conversation)
  │
  ├── Web UI (WebSocket)
  ├── Telegram (Bot API via grammY)
  └── WhatsApp (Web API via Baileys)
  │
  ▼
osmoda-bridge → agentd / keyd / watch / routines
  │
  ▼
agentd ledger (every message logged with channel source)
```

One conversation. Multiple windows into it.

---

## Troubleshooting

**Bot doesn't respond after setup:**
Tell osModa: "Check if the gateway restarted properly" — it will check the logs for you.

Or manually:
```bash
journalctl -u osmoda-gateway --since "5 min ago"
```

**WhatsApp QR expired:**
Tell osModa: "I need a new WhatsApp QR code" — it will restart the gateway and show you the new one.

**Wrong token:**
Tell osModa: "The Telegram token is wrong, here's the new one: ..." — it overwrites the file and restarts.
