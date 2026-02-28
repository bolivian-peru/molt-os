# Getting Started with osModa

Your first 10 minutes with an AI-managed server.

## Prerequisites

- A NixOS machine (or willingness to convert — see [Quickstart](../README.md#quickstart))
- An Anthropic API key (for the AI gateway)

---

## Step 1: Install

### On an existing NixOS system

Add osModa to your flake:

```nix
# flake.nix
{
  inputs.os-moda.url = "github:bolivian-peru/os-moda";

  outputs = { self, nixpkgs, os-moda, ... }: {
    nixosConfigurations.myserver = nixpkgs.lib.nixosSystem {
      modules = [
        os-moda.nixosModules.default
        {
          services.osmoda.enable = true;
        }
      ];
    };
  };
}
```

```bash
sudo nixos-rebuild switch
```

### On Ubuntu/Debian (converts to NixOS)

```bash
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | sudo bash
```

> **Warning:** This converts your OS to NixOS. Use on fresh/disposable servers only.

### Verify installation

After install, all 9 daemons should be running:

```bash
curl -s --unix-socket /run/osmoda/agentd.sock http://localhost/health | jq
```

Expected output:

```json
{
  "hostname": "myserver",
  "uptime_seconds": 1234,
  "cpu_usage": [3.2, 1.5, 2.8, 4.1],
  "memory_total": 8589934592,
  "memory_used": 2147483648,
  "memory_available": 6442450944,
  "load_average": { "one": 0.15, "five": 0.10, "fifteen": 0.08 },
  "disks": [
    { "mount": "/", "total": 107374182400, "used": 32212254720, "available": 75161927680 }
  ]
}
```

---

## Step 2: Open the Web Chat

Navigate to the gateway URL shown after install (default: `http://localhost:18789`).

The AI will greet you with a health check summary. This is your primary interface — you talk to the server in plain language, and it responds with structured data and actions.

---

## Step 3: Ask "How's my server doing?"

Type this into the chat. The AI will:

1. Call `system_health` to get CPU, RAM, disk, and load averages
2. Call `system_discover` to find all running services and listening ports
3. Present a summary like:

```
Your server looks healthy.

CPU: 3.2% average across 4 cores
RAM: 2.0 / 8.0 GB (25%)
Disk: 30 / 100 GB (30%)
Load: 0.15 (last 1 min)

Running services:
  sshd         — active (port 22)
  nginx        — active (port 80, 443)
  osmoda-agentd — active (socket)
  osmoda-watch  — active (socket)
  ... and 7 more osModa daemons

No issues detected.
```

Every query the AI runs is logged to the hash-chained audit ledger. You can verify this anytime:

```bash
agentctl verify-ledger --state-dir /var/lib/osmoda
```

---

## Step 4: Set Up Telegram

Say: **"Set up Telegram so I can message you from my phone"**

The AI will walk you through:

1. Open Telegram, search for **@BotFather**
2. Send `/newbot` and pick a name for your bot
3. Copy the bot token (looks like `123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11`)
4. Paste it into the chat — the AI saves it securely and configures the gateway
5. Find your bot on Telegram and send it a message

From now on, you can manage your server from your phone. The mobile agent (Claude Sonnet) handles Telegram messages with full access — it can do everything the web chat can (deploy apps, run commands, manage services), just with concise, phone-friendly responses.

---

## Step 5: Ask "What can you do?"

The AI will explain its capabilities based on what's actually running on your system. It won't dump a generic feature list — it'll tell you what it can see and control right now.

Common things to try:

- **"What's using the most CPU?"** — runs `system_query` with process sort
- **"Show me the last hour of nginx logs"** — runs `journal_logs` filtered by unit
- **"Is anything listening on port 5432?"** — runs `system_discover`
- **"Set up a watcher for nginx"** — creates an autopilot health check that restarts nginx if it goes down

---

## Step 6: Try Something Real

Say: **"Install nginx and set up a reverse proxy for port 3000"**

The AI will:

1. Edit your NixOS configuration to enable nginx with a reverse proxy
2. Show you the diff before applying
3. Deploy via SafeSwitch — the change goes through a probation period with health checks
4. If nginx starts and responds correctly, the change commits
5. If anything fails, automatic rollback to the previous system state

You'll see something like:

```
I'll add nginx as a reverse proxy for port 3000. Here's the NixOS config change:

  services.nginx.enable = true;
  services.nginx.virtualHosts."localhost" = {
    locations."/" = {
      proxyPass = "http://127.0.0.1:3000";
    };
  };

This will be deployed via SafeSwitch with a 60-second health check window.
If nginx doesn't start correctly, the system rolls back automatically.

Apply this change?
```

---

## What's Next

- **Set up monitoring** — "Watch nginx and restart it if it goes down"
- **Deploy an app** — "Deploy my Node.js API at /home/user/api on port 3000"
- **Connect to another server** — "Create a mesh invite so my other server can connect"
- **Set up remote access** — "Enable Cloudflare Tunnel so I can access this from anywhere"
- **Check the audit trail** — "Show me everything that changed today"

---

## Useful Commands

These work outside the AI, directly on the command line:

```bash
# System health (structured JSON)
curl -s --unix-socket /run/osmoda/agentd.sock http://localhost/health | jq

# Verify audit ledger integrity
agentctl verify-ledger --state-dir /var/lib/osmoda

# View recent audit events
agentctl events --state-dir /var/lib/osmoda --limit 20

# Emergency rollback (bypasses AI)
# Use safety_rollback in the chat, or directly:
sudo nixos-rebuild --rollback switch
```

---

## Troubleshooting

**Daemons not starting?** Check systemd:
```bash
systemctl status osmoda-agentd osmoda-watch osmoda-routines
journalctl -u osmoda-agentd --since '5 min ago'
```

**Can't reach the web chat?** The gateway listens on port 18789 by default:
```bash
curl -s http://localhost:18789/health
```

**Need to start over?** NixOS rollback is always available:
```bash
sudo nixos-rebuild --rollback switch
```
