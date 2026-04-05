# 🎬 osModa Self-Healing Demo Guide

**BOUNTY:** Film "I Deleted My Nginx Config — Watch What Happens Next"

**REWARD:** 1 SOL

**Difficulty:** ⭐⭐☆☆☆ (Easy with this guide)

---

## 📋 Overview

This guide walks you through setting up and filming a dramatic demo of osModa's self-healing capabilities. You'll:

1. Set up a real application stack (PostgreSQL + Todo API + Nginx)
2. Break something dramatic (delete nginx config!)
3. Film osModa automatically detecting and healing the damage
4. Show the audit log proving what happened

**Total time:** ~30 minutes setup + 10 minutes filming

---

## 🚀 Quick Start

### Prerequisites

- A server with osModa already installed (Ubuntu/Debian → NixOS)
- Root access
- Camera or screen recording software
- ~30 minutes

### Step 1: Run the Demo Setup

```bash
cd /opt/osmoda
sudo ./scripts/demo-setup.sh
```

This will:
- ✅ Install and configure PostgreSQL
- ✅ Create and deploy the Todo API app
- ✅ Set up Nginx reverse proxy
- ✅ Add sample data
- ✅ Verify everything works

**Wait for:** `✅ Demo Setup Complete!` message

### Step 2: Verify Everything Works

Before filming, make sure everything is working:

```bash
# Test the health endpoint
curl http://localhost:3000/health

# Test the todos endpoint
curl http://localhost/todos

# Test via nginx (port 80)
curl http://localhost/
```

You should see JSON responses showing the app is healthy.

---

## 🎥 Filming the Demo

### Camera Setup

**Important:** Film your actual screen with a camera (phone on tripod), not just screen recording. This makes it more authentic and verifiable.

**Recommended setup:**
- Phone on tripod filming your laptop/monitor
- 1080p resolution minimum
- Good lighting so terminal is readable
- Quiet environment for voiceover (optional)

### The Script (60 seconds)

#### 0:00-0:03 — Hook
**Show:** Terminal with osModa running
**Say:** "What happens when I delete my server's nginx config on an AI-managed server?"

#### 0:03-0:10 — Show It Working
**Show:** 
```bash
curl http://localhost/todos
```
**Say:** "Here's my todo app running behind nginx. Everything's working fine."

#### 0:10-0:15 — The Destruction
**Show:** Terminal, ready to run the command
**Say:** "Now watch what happens when I do this..."
**Run:**
```bash
sudo rm -rf /etc/nginx/sites-enabled/*
sudo rm -rf /etc/nginx/sites-available/*
```

#### 0:15-0:20 — Show It's Broken
**Show:**
```bash
curl http://localhost/
# Should fail with connection refused
```
**Say:** "Nginx is dead. Or is it?"

#### 0:20-0:40 — The Recovery
**Show:** Terminal, waiting
**Say:** "osModa's watchdog just detected the failure. The AI is diagnosing... and now it's fixing."

**Show the recovery:**
```bash
systemctl status nginx
# Should show it's back online!
```

#### 0:40-0:50 — The Proof
**Show:**
```bash
curl http://localhost/todos
# Should work again!
```
**Say:** "Nginx is back. Total downtime: [show the timestamp]. The audit log has everything."

#### 0:50-1:00 — The Punchline
**Show:** Audit log or osModa status
**Say:** "Try this on your Ubuntu server. I dare you. Deploy at spawn.os.moda"

**End frame:** Show `spawn.os.moda` URL for 3 seconds

---

## 🔥 Alternative Break Scenarios

If you want to film different demos, here are other options:

### 1. Kill PostgreSQL
```bash
sudo systemctl stop postgresql
# Watch osModa restart it automatically
```

### 2. Kill the Todo App
```bash
sudo systemctl stop todo-app
# Watch osModa restart it automatically
```

### 3. Memory Stress Test
```bash
sudo ./scripts/demo-break.sh stress
# Watch osModa detect and kill the stress process
```

### 4. Nuclear Option (All Services)
```bash
sudo systemctl stop postgresql todo-app nginx
# Watch osModa heal everything
```

---

## 📊 Show the Audit Log

After the recovery, show the audit trail:

```bash
# List recent audit entries
ls -la /var/lib/osmoda/audit/

# Show the latest entries
tail -50 /var/lib/osmoda/audit/*.json

# Or use agentctl if available
agentctl audit --recent
```

**What to highlight:**
- The timestamp of the incident
- The AI's diagnosis
- The action taken (rollback, restart, etc.)
- The hash chain proving tamper-evidence

---

## 📹 Video Format Requirements

### Vertical (9:16) — TikTok / Reels / Shorts
- **Resolution:** 1080x1920 minimum
- **Duration:** 45-90 seconds
- **Style:** Fast cuts, big text overlays
- **Captions:** Burned-in subtitles
- **Post to:** TikTok, Instagram Reels, YouTube Shorts

### Landscape (16:9) — YouTube / LinkedIn / Twitter
- **Resolution:** 1920x1080 minimum
- **Duration:** 60-120 seconds
- **Style:** Calm walkthrough, terminal readable
- **Thumbnail:** Custom with big text
- **Post to:** YouTube (main), LinkedIn, Twitter/X

---

## ✅ Submission Checklist

Before submitting:

- [ ] Real camera filming real screen (not just screen recording)
- [ ] Uncut deletion → recovery sequence
- [ ] Show the audit log after recovery
- [ ] Show the timestamp / total downtime
- [ ] Upload to YouTube (public, not unlisted)
- [ ] Minimum 720p quality
- [ ] Link osModa repo in description
- [ ] Link spawn.os.moda in description
- [ ] Post the YouTube link in issue #2

### Video Description Template

```markdown
I tested osModa's self-healing by deleting my nginx config. Watch what happens.

osModa is a NixOS distribution where AI IS the operating system.
- 10 Rust daemons
- 83 tools
- 194 tests
- Zero cloud dependencies

Deploy your own: https://spawn.os.moda
GitHub: https://github.com/bolivian-peru/os-moda

#osModa #AIagents #NixOS #Rust #SelfHealing #DevOps #ServerManagement
```

---

## 🐛 Troubleshooting

### "osModa isn't healing automatically"

1. Check if agentd is running: `systemctl status agentd`
2. Check logs: `journalctl -u agentd -f`
3. Verify watchers are configured
4. Open an issue with the error output

### "PostgreSQL won't start"

1. Check logs: `journalctl -u postgresql -f`
2. Verify disk space: `df -h`
3. Check permissions: `ls -la /var/lib/postgresql/`

### "Nginx config won't rollback"

1. Check if NixOS rollback is available: `nix-env --list-generations`
2. Manually trigger rollback: `nix-env --rollback`
3. Check osModa logs for errors

---

## 💡 Pro Tips

1. **Do a test run first** — Practice the demo once before filming
2. **Keep terminal large** — Use 24pt+ font for readability
3. **Show timestamps** — Run `date` before and after to show downtime
4. **Multiple takes OK** — You can re-film, just keep the destruction→recovery uncut
5. **Authenticity > polish** — Showing bugs and fixes is MORE valuable than perfect demo

---

## 🎯 What Judges Look For

✅ **Authenticity** — Real server, real break, real recovery
✅ **Clarity** — Easy to understand what's happening
✅ **Drama** — The destruction should feel real and dangerous
✅ **Proof** — Show the audit log and timestamps
✅ **Call to action** — Clear link to spawn.os.moda

❌ **Fake demos** — Scripted or faked recovery
❌ **Unclear** — Can't tell what broke or what fixed it
❌ **No proof** — No audit log or timestamps shown
❌ **Too long** — Over 2 minutes loses attention

---

## 📞 Need Help?

If you hit bugs during filming:

1. **Open an issue** with `[BUG]` in the title
2. **Include:** OS, hardware, error output, steps to reproduce
3. **Tag it:** "Found while working on #2"
4. **We'll fix it live** — Maintainers will debug with you in real-time

**Remember:** Bugs are expected! This is also a testing bounty. Even if your demo shows a rough journey, you can still earn the SOL.

---

## 🏆 Ready to Submit?

1. Film the demo ✅
2. Upload to YouTube ✅
3. Post the link in [issue #2](https://github.com/bolivian-peru/os-moda/issues/2) ✅
4. Receive 1 SOL! 🎉

**Good luck! 🚀**

---

*Last updated: 2026-03-24*
*For osModa version: 0.1.0*
