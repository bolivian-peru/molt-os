# AgentOS Demo Script — 2 Minute Video

## Setup Before Recording

```bash
# SSH into Hetzner with tunnel
ssh -i .keys/agentos_hetzner -L 18789:localhost:18789 root@<your-server-ip>

# Verify everything is running
curl -s --unix-socket /run/agentos/agentd.sock http://l/health | jq .status
systemctl status openclaw-gateway --no-pager | head -5

# Open http://localhost:18789 in browser
# Set heartbeat to 1 minute for demo (normally 30 min)
```

## Recording Software
- OBS or screen.studio
- Browser full screen on localhost:18789
- Terminal split-screen for showing server side

---

## SCENE 1: "Talk to Your Server" (30 seconds)

**Show:** Browser on localhost:18789, OpenClaw chat interface

**Type in chat:**
> What's the current state of my server?

**Expected response:** Thorox runs system_health, shows CPU/RAM/disk/uptime in a clean summary.

**Then type:**
> How many failed SSH login attempts in the last 24 hours? Who's trying to break in?

**Expected response:** Thorox runs journal_logs for sshd, counts failed attempts, identifies IPs.

**Narration:** "I stopped SSHing into my servers. Now I just ask them questions."

---

## SCENE 2: "Natural Language DevOps" (30 seconds)

**Type in chat:**
> Install nginx and set it up as a reverse proxy for my app on port 3000 with HTTPS

**Expected response:** Thorox generates NixOS config diff, explains each line, asks for approval.

**Type:**
> Yes, apply it

**Expected response:** Thorox runs nixos-rebuild switch, verifies nginx is running, confirms generation number.

**Narration:** "No more writing NixOS configs by hand. No more Ansible playbooks. Just tell it what you want."

---

## SCENE 3: "Self-Healing" (45 seconds) — THE MONEY SHOT

**Setup:** Have nginx already running from Scene 2 (or pre-installed).

**In a terminal (visible to camera):**
```bash
# Deliberately break nginx
systemctl stop nginx
```

**Wait for heartbeat to detect it (set to 1 min for demo).**

**Show the chat:** Thorox proactively messages:
> I detected that nginx went down at [timestamp]. Investigating...
>
> The service was stopped manually (no crash, no config error).
> Restarting nginx... Done. All health checks pass.
>
> Audit entry #[N] recorded with hash chain verification.

**Narration:** "3 AM, your server breaks, you're asleep. AgentOS detects it, diagnoses it, fixes it, and logs everything to a tamper-proof audit trail. You wake up to a Telegram message: 'Fixed it. Here's what happened.'"

---

## SCENE 4: "The Audit Trail" (15 seconds)

**Type in chat:**
> Show me the audit ledger — last 10 events

**Expected response:** Hash-chained event list showing the self-healing incident, each with SHA-256 hashes.

**Narration:** "Every action is hash-chained. Tamper-proof. SOC2-ready. You can prove exactly what your AI did, when, and why."

---

## CLOSING (10 seconds)

Show terminal:
```bash
curl -fsSL https://agentos.dev/install | sudo bash
```

**Text overlay:** "AgentOS — the server that thinks. Open source. One command to install."

**Text overlay:** "github.com/moltOS/molt-os"

---

## Alternative Scenes (if time allows)

### Morning Briefing
Configure heartbeat to run morning briefing, show the output:
```
Good morning. Infrastructure report:
🟢 All services healthy
📊 CPU avg 11% | RAM 4.1/7.6 GB | Disk 38%
🔒 7 failed SSH attempts, all blocked
💰 $0.33/day
All systems nominal.
```

### Security Score
> Run a security audit on this server

Shows the 100-point scoring with auto-fixes and recommendations.

### Time-Travel Debugging
> What changed on this server in the last week?

Thorox correlates NixOS generations + audit ledger + journal logs into a timeline.

---

## Tips for Recording

1. **Speed up typing** — type fast, edit out pauses in post
2. **Pre-warm the cache** — run each prompt once before recording so responses are fast
3. **Keep the terminal visible** — split screen browser + terminal for credibility
4. **Music** — subtle lo-fi background, cut during narration
5. **Length** — 60 seconds for Twitter/X, 120 seconds for Product Hunt, 10 min for YouTube
