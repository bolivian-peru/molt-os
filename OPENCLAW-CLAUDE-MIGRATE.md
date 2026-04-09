---
name: openclaw-to-claude-code
description: >
  Migrate an OpenClaw deployment from API-credit billing to a Claude Code
  runtime authenticated with a Max-plan OAuth token. Replaces every
  agentd / openclaw-gateway invocation point with `claude -p` shell
  wrappers, preserves all existing skills, adds a Telegram bridge with
  per-conversation session resumption, and wires the whole thing into
  systemd (NixOS module + non-NixOS variant). Reference implementation
  is the VPS3 (clawdbot3 / 76.13.250.48) deployment.
trigger: |
  Use this skill when the user asks about:
  - Moving an OpenClaw / agentd box off the Anthropic API ($) onto a
    Max-plan subscription
  - Replacing `openclaw run` / `agentd` invocations with `claude -p`
  - Authenticating Claude Code on a headless VPS with an OAuth token
  - Wiring Claude Code into systemd timers (heartbeat, monitor, review)
  - Building a Telegram bot that talks to Claude Code with multi-conv
    memory
  - Keeping the existing `~/.openclaw/skills/*` tree usable from the
    new runtime
allowed-tools:
  - "Bash(*)"
  - "Read(*)"
  - "Write(*)"
  - "Edit(*)"
requires:
  bins:
    - bash
    - curl
    - node    # >= 20
    - npm
    - jq
    - systemctl
  runtime:
    - "Anthropic Claude Max plan account (for OAuth token)"
    - "Existing OpenClaw deployment under ~/.openclaw/"
    - "Telegram bot token + chat id (optional, for bridge)"
---

# OpenClaw → Claude Code Runtime Migration

> **Two migration paths exist. Choose one:**
>
> **Path A — osModa automated (recommended for spawn.os.moda servers):**
> Run `install.sh --runtime claude-code` (now the default). This installs
> `osmoda-gateway` (Claude Code SDK, port 18789) with 91 MCP tools,
> Telegram webhook, multi-agent routing. All daemons stay running.
> See: `packages/osmoda-gateway/`, `packages/osmoda-mcp-bridge/`.
>
> **Path B — Manual wrapper migration (this document):**
> For custom deployments (trading bots, single-purpose VPS) where you
> want per-task `claude -p` shell wrappers instead of the full osModa
> gateway. Disables osmoda-gateway and replaces it with individual
> systemd timers. **Do NOT mix Path A and Path B on the same server.**

This skill is the tested, reproducible procedure used to convert an
OpenClaw + agentd deployment (which consumes Anthropic API credits per
token) into a `claude -p` based runtime that bills against a Claude Max
subscription instead. Every shell wrapper, systemd unit, and config file
referenced below is taken verbatim from the working VPS3 deployment.

## Why Migrate

OpenClaw routes every agent turn through the Anthropic API and bills the
user per input/output token. A single market-maker review every 2h can
burn $20–$60/day depending on context size. Claude Code with a Max-plan
OAuth token bills the same workload against the plan's quota
(`default_claude_max_20x` rate-limit tier on the 20× plan), which is a
fixed monthly cost. The migration is non-destructive: the original
`~/.openclaw/skills/*` tree, scripts, configs, and state files all stay
in place. Only the **invocation surface** changes — every place that
used to shell out to `openclaw <skill>` becomes `claude -p "<prompt>"`.

## High-level Architecture

```
                        ┌──────────────────────────────┐
                        │  /root/.claude/               │
                        │   .credentials.json (OAuth)   │
                        │   settings.json (perms)       │
                        │   projects/-root-workspace/   │  ← session jsonl per conv
                        └──────────────┬───────────────┘
                                       │
   /root/workspace/  (always cwd)      │
   ├─ CLAUDE.md       (auto-loaded)    │
   ├─ skills/         (legacy openclaw)│
   ├─ .claude/        (project perms)  │
   └─ .openclaw → /root/.openclaw      │
                                       │
                   ┌───────────────────┴────────────────────┐
                   │                                        │
        /opt/claude-code/                          /root/.openclaw/skills/
        ├─ heartbeat.sh   ───┐                     ├─ cs2-marketmaker/
        ├─ mm-review.sh   ───┤   each cron wrapper ├─ defi-tracker/
        ├─ rewards-*.sh   ───┤   builds a prompt   ├─ anti-detect-browser/
        ├─ telegram-bridge.sh│   and pipes it into └─ erudite/
        └─ node_modules/    ▼          claude -p
                  @anthropic-ai/claude-code  ──→  Anthropic OAuth
                                                  (Max plan quota)
                                       │
                                       ▼
                            /etc/nixos/claude-code.nix
                            (or /etc/systemd/system/*.service
                             on non-NixOS hosts)
```

## Phase 0 — Inventory the Source Box

Before touching anything, capture what the existing OpenClaw setup is
doing so nothing is lost in the swap.

```bash
# 1. List every openclaw / agentd invocation point
systemctl list-unit-files --no-pager | grep -Ei 'openclaw|agentd|osmoda'
crontab -l 2>/dev/null
ls -la ~/.openclaw/skills/

# 2. Capture the working state (pause-safe, no stop yet)
ls ~/.openclaw/*/state/   ~/.openclaw/*/config/ 2>/dev/null
find ~/.openclaw -name 'settings.json' -maxdepth 4

# 3. Note any secrets the skills need (telegram, RPC keys, wallet keys)
find ~/.openclaw -name '*.key' -o -name 'secrets*' -o -name '.env*' 2>/dev/null
ls /var/lib/osmoda/secrets/ 2>/dev/null
```

Write the output to a scratch file. You will reference it in Phase 5
when rewriting the wrappers.

## Phase 1 — Install Claude Code

Claude Code is shipped as an npm package (`@anthropic-ai/claude-code`).
We deliberately install it under `/opt/claude-code` rather than globally
so the systemd units can pin a specific version.

```bash
# Requires Node.js >= 20. On NixOS this is in nixpkgs; on Debian use
# nodesource. Verify first:
node --version    # must be >= v20

# Create the install dir
sudo mkdir -p /opt/claude-code
sudo chown root:root /opt/claude-code
cd /opt/claude-code

# Pin the version in package.json so reinstalls are deterministic
sudo tee package.json >/dev/null <<'JSON'
{
  "name": "claude-code",
  "version": "1.0.0",
  "private": true,
  "dependencies": {
    "@anthropic-ai/claude-code": "^2.1.92"
  }
}
JSON

sudo npm install

# Symlink so PATH=/root/.local/bin:... resolves
sudo mkdir -p /root/.local/bin
sudo ln -sf /opt/claude-code/node_modules/.bin/claude /root/.local/bin/claude

# Smoke test
/root/.local/bin/claude --version
# Expected: 2.1.92 (Claude Code)
```

## Phase 2 — OAuth Token (Max plan auth)

This is the only step that varies by host. Claude Code stores its OAuth
material in `~/.claude/.credentials.json` (mode 600). The CLI can
populate it interactively, or you can copy a pre-existing file from a
machine where you've already authenticated.

### Option A — Interactive (host has a browser-capable terminal)

```bash
HOME=/root /root/.local/bin/claude    # interactive REPL
# Inside the REPL:
/login
# Follow the URL it prints; complete the OAuth flow in your browser;
# paste the returned code back into the REPL.
# The CLI writes /root/.claude/.credentials.json automatically.
/exit
```

### Option B — Headless VPS (most common)

1. Run `claude /login` on a workstation where you can open a browser.
2. Authenticate with the **same Anthropic account that owns the Max
   plan**. Confirm by inspecting the freshly written file:

   ```bash
   cat ~/.claude/.credentials.json | jq .claudeAiOauth.subscriptionType
   # → "max"
   cat ~/.claude/.credentials.json | jq .claudeAiOauth.rateLimitTier
   # → "default_claude_max_20x" (or _5x depending on plan tier)
   ```

3. Copy the file to the target VPS, preserving mode 600:

   ```bash
   scp ~/.claude/.credentials.json vps:/root/.claude/.credentials.json
   ssh vps "chmod 600 /root/.claude/.credentials.json && \
            chown root:root /root/.claude/.credentials.json"
   ```

The expected file shape (token values redacted):

```json
{
  "claudeAiOauth": {
    "accessToken":  "sk-ant-oat01-…",
    "refreshToken": "sk-ant-ort01-…",
    "expiresAt":    1775774290080,
    "scopes": [
      "user:file_upload",
      "user:inference",
      "user:mcp_servers",
      "user:profile",
      "user:sessions:claude_code"
    ],
    "subscriptionType": "max",
    "rateLimitTier":    "default_claude_max_20x"
  }
}
```

The CLI automatically refreshes `accessToken` using `refreshToken`
before expiry — no cron job is needed. **Do NOT also set
`ANTHROPIC_API_KEY`** anywhere in the environment; if both are present
the API key wins and you fall back to credit billing. Audit the systemd
units in Phase 6 for stray `ANTHROPIC_API_KEY` exports.

## Phase 3 — Workspace + CLAUDE.md

Claude Code looks for `CLAUDE.md` in the current working directory at
launch and prepends its contents to the system prompt. This is how the
runtime "knows" about the host's services, paths, and rules without
having to be told every invocation.

```bash
sudo mkdir -p /root/workspace
cd /root/workspace

# Symlink the legacy openclaw tree so existing skill scripts are
# reachable with the same paths they had under openclaw
sudo ln -sf /root/.openclaw .openclaw
sudo ln -sf /opt/claude-code claude-code

# CLAUDE.md (auto-loaded). Customize service names, paths, wallet, etc.
sudo tee /root/workspace/CLAUDE.md >/dev/null <<'MD'
# <hostname> — short host description

## System
- Distro / version
- Config: /etc/<distro-config-path>
- Node.js, jq, etc. installed via package manager

## Services
- Whatever long-running daemons exist on this box

## Skills (scripts at ~/.openclaw/skills/)

### <skill-name>
Short description.
- Scripts: ~/.openclaw/skills/<skill>/scripts/
- Config:  ~/.openclaw/<skill>/config/settings.json
- State:   ~/.openclaw/<skill>/state/
- Wallet / API endpoints / etc.

## Key Paths
- Workspace: /root/workspace/
- Skills:    /root/.openclaw/skills/

## Periodic Tasks
- heartbeat: every 5 min
- review:    every 2 h
- monitor:   every 10 min

## Rules
- Never modify <distro-config> without explicit user approval
- Never push to git without explicit approval
- <other invariants>
MD
```

The directory name is encoded into the session-storage path. With
`cwd = /root/workspace`, sessions land in
`/root/.claude/projects/-root-workspace/<session-id>.jsonl`. This
matters for the Telegram bridge in Phase 7.

## Phase 4 — Settings (permissions + MCP)

Two settings files are honored: a global one at `~/.claude/settings.json`
and an optional per-project one at `<cwd>/.claude/settings.json`. They
merge, project-level overrides global. For an unattended VPS we usually
want every tool unconditionally allowed.

```bash
# Global
sudo mkdir -p /root/.claude
sudo tee /root/.claude/settings.json >/dev/null <<'JSON'
{
  "permissions": {
    "allow": [
      "Bash(*)",
      "Read(*)",
      "Write(*)",
      "Edit(*)",
      "Glob(*)",
      "Grep(*)",
      "Agent(*)"
    ],
    "deny": []
  },
  "enableAllProjectMcpServers": true
}
JSON

# Per-project (lives next to CLAUDE.md so the rules travel with the cwd)
sudo mkdir -p /root/workspace/.claude
sudo tee /root/workspace/.claude/settings.json >/dev/null <<'JSON'
{
  "permissions": {
    "allow": [
      "Bash(*)", "Read(*)", "Write(*)", "Edit(*)",
      "Glob(*)", "Grep(*)", "Agent(*)"
    ],
    "deny": []
  }
}
JSON
```

If the OpenClaw skills relied on MCP servers (gmail, calendar, etc.),
add their configs to `<cwd>/.mcp.json`. The cache file
`/root/.claude/mcp-needs-auth-cache.json` is regenerated by the CLI on
first run and tracks which servers still need OAuth.

## Phase 5 — Convert Skill Wrappers

The migration's heart: each `openclaw <skill>` invocation becomes a tiny
shell wrapper that gathers state, builds a prompt, and pipes it into
`claude -p`. Drop these in `/opt/claude-code/`. Three patterns cover
~95% of cases.

### Pattern A — Pure compute (no LLM needed in steady-state)

`heartbeat.sh` only escalates to Claude when something is wrong.

```bash
sudo tee /opt/claude-code/heartbeat.sh >/dev/null <<'SH'
#!/usr/bin/env bash
# Cheap health check; escalates to claude -p only on alert.
export PATH=/root/.local/bin:/run/current-system/sw/bin:$PATH
export HOME=/root

LOG="/root/.openclaw/<skill>/state/heartbeat.log"

cpu=$(awk '{print $1}' /proc/loadavg)
mem=$(free | awk '/Mem:/{printf "%.0f", $3/$2*100}')
disk=$(df / | awk 'NR==2{print $5}' | tr -d %)
down=""
for svc in <critical-svc-1> <critical-svc-2>; do
    systemctl is-active "$svc" >/dev/null 2>&1 || down="$down $svc"
done
echo "$(date -u +%FT%TZ) load=$cpu mem=${mem}% disk=${disk}%${down:+ DOWN:$down}" >> "$LOG"

# LLM only when something is wrong
if (( mem > 85 )) || (( disk > 90 )) || [[ -n "$down" ]]; then
    claude -p "ALERT: load=$cpu mem=${mem}% disk=${disk}% down=$down. \
Investigate and report what action is needed." \
        --max-turns 3 >> "$LOG" 2>&1
fi
SH
sudo chmod +x /opt/claude-code/heartbeat.sh
```

### Pattern B — Periodic LLM review with structured Telegram tail

`mm-review.sh` runs every 2h, reads state files, asks Claude to
analyze + fix config + emit a `---TELEGRAM---` block, then forwards
that block to Telegram.

```bash
sudo tee /opt/claude-code/mm-review.sh >/dev/null <<'SH'
#!/usr/bin/env bash
export PATH=/root/.local/bin:/run/current-system/sw/bin:$PATH
export HOME=/root

STATE_DIR="/root/.openclaw/<skill>/state"
CONFIG="/root/.openclaw/<skill>/config/settings.json"
LOG="$STATE_DIR/review.log"
BOT_TOKEN=$(cat /var/lib/osmoda/secrets/telegram-bot-token 2>/dev/null)
CHAT_ID="<your-chat-id>"

send_tg() {
    [[ -z "$BOT_TOKEN" ]] && return
    curl -s "https://api.telegram.org/bot${BOT_TOKEN}/sendMessage" \
        -d "chat_id=$CHAT_ID" \
        --data-urlencode "text=$1" >/dev/null 2>&1
}

# Compose context from state files (jq / node -e are both fine)
DAILY=$(cat "$STATE_DIR/daily-pnl.json" 2>/dev/null)
POSITIONS=$(jq -c '.markets | length' "$STATE_DIR/positions.json" 2>/dev/null)
RECENT=$(tail -25 "$STATE_DIR/run.log" 2>/dev/null)

PROMPT="You are the automated reviewer for the <skill> system.

CURRENT STATE:
- Daily: $DAILY
- Positions: $POSITIONS
- Recent log:
$RECENT

TASKS:
1. Analyze performance.
2. If you see a clear improvement, edit settings.json and explain.
3. Emit a structured Telegram block at the end:

---TELEGRAM---
3-5 line summary the user will see in chat
---END---

Rules: you can edit settings.json, you cannot restart services."

cd /root/workspace
RESPONSE=$(claude -p "$PROMPT" 2>/dev/null)
echo "$(date -u +%FT%TZ) review" >> "$LOG"
echo "$RESPONSE" >> "$LOG"

TG_MSG=$(echo "$RESPONSE" | sed -n '/---TELEGRAM---/,/---END---/p' | grep -v '^---')
[[ -n "$TG_MSG" ]] && send_tg "🔍 Review $(date -u +%H:%M)
$TG_MSG"
SH
sudo chmod +x /opt/claude-code/mm-review.sh
```

### Pattern C — Pure node.js worker (no Claude needed)

Some skills are background workers that already run a node process —
e.g. an order placer or RPC poller. Wrap them in a thin systemd-friendly
shell so they get the same env + restart policy.

```bash
sudo tee /opt/claude-code/rewards-quoter.sh >/dev/null <<'SH'
#!/usr/bin/env bash
export PATH=/root/.local/bin:/run/current-system/sw/bin:$PATH
export HOME=/root
cd /root/.openclaw/skills/<skill>
exec node scripts/rewards-quoter.mjs
SH
sudo chmod +x /opt/claude-code/rewards-quoter.sh
```

> ⚠ Pattern C does not invoke `claude` at all — this is intentional. Not
> every workload needs an LLM in the hot path. Mixing C with A/B in the
> same systemd module gives you a Claude-augmented runtime where the
> LLM only spends quota on the work that actually benefits from it.

### Conversion checklist (per existing skill)

For each `openclaw <skill>` invocation in the source inventory:

| Old (OpenClaw)                              | New (Claude Code)                         |
|---|---|
| `openclaw run <skill> --task X`             | wrapper in `/opt/claude-code/<skill>.sh`  |
| `agentd send-prompt …`                      | `claude -p "$PROMPT"`                     |
| reads `~/.openclaw/skills/<skill>/SKILL.md` | unchanged — `claude -p` reads it via cwd  |
| posts results to telegram via openclaw plugin | `curl … sendMessage` inside the wrapper |
| streams tool calls back through agentd       | `claude -p` returns final stdout only — capture, parse, forward |

## Phase 6 — Systemd Integration

Wire each wrapper into a systemd timer (oneshot) or service (long-lived).

### NixOS variant — `/etc/nixos/claude-code.nix`

This is the exact module shape used on the reference VPS3. Drop it into
`/etc/nixos/`, then `import` it from `configuration.nix`:

```nix
{ pkgs, ... }:
{
  systemd.services.claude-heartbeat = {
    description = "Claude Code Heartbeat";
    serviceConfig = {
      Type = "oneshot";
      ExecStart = "/opt/claude-code/heartbeat.sh";
      Environment = [
        "PATH=/root/.local/bin:/run/current-system/sw/bin"
        "HOME=/root"
      ];
    };
  };
  systemd.timers.claude-heartbeat = {
    wantedBy = [ "timers.target" ];
    timerConfig = { OnCalendar = "*:0/5"; Persistent = true; };
  };

  systemd.services.claude-mm-review = {
    description = "Periodic Trading Review";
    serviceConfig = {
      Type = "oneshot";
      ExecStart = "/opt/claude-code/mm-review.sh";
      TimeoutStartSec = 600;
      Environment = [
        "PATH=/root/.local/bin:/run/current-system/sw/bin"
        "HOME=/root"
      ];
    };
  };
  systemd.timers.claude-mm-review = {
    wantedBy = [ "timers.target" ];
    timerConfig = { OnCalendar = "*:0/120"; Persistent = true; };
  };

  systemd.services.claude-telegram = {
    description = "Claude Code Telegram Bridge";
    wantedBy = [ "multi-user.target" ];
    after = [ "network-online.target" ];
    wants = [ "network-online.target" ];
    serviceConfig = {
      Type = "simple";
      ExecStart = "/opt/claude-code/telegram-bridge.sh";
      Restart = "always";
      RestartSec = 10;
      Environment = [
        "PATH=/root/.local/bin:/run/current-system/sw/bin"
        "HOME=/root"
      ];
    };
  };

  systemd.services.rewards-quoter = {
    description = "Background worker (no LLM)";
    wantedBy = [ "multi-user.target" ];
    after = [ "network-online.target" ];
    wants = [ "network-online.target" ];
    serviceConfig = {
      Type = "simple";
      ExecStart = "/opt/claude-code/rewards-quoter.sh";
      Restart = "always";
      RestartSec = 30;
      MemoryMax = "256M";
      Environment = [
        "PATH=/root/.local/bin:/run/current-system/sw/bin"
        "HOME=/root"
      ];
    };
  };

  # PATH B ONLY: If using manual wrappers (this document), disable the
  # osModa gateway so it doesn't fight over Telegram polling / ports.
  # WARNING: Do NOT disable this on Path A (osModa automated) servers —
  # osmoda-gateway IS the Claude Code runtime on those boxes.
  # systemd.services.osmoda-gateway.enable = false;  # uncomment for Path B only
}
```

Reference it from `configuration.nix`:

```nix
{ ... }:
{
  imports = [
    ./hardware-configuration.nix
    ./networking.nix
    ./claude-code.nix      # ← add
  ];
  # …rest of config…
}
```

Apply: `sudo nixos-rebuild switch`. NixOS will start the new units and
disable `osmoda-gateway` in a single transaction.

> 🪤 **NixOS gotcha:** `/etc/systemd/system/` is read-only. You cannot
> `systemctl edit` or drop unit files into it directly — every unit
> must come from the Nix module above. Same applies to `systemctl
> enable/disable`: toggle by changing `wantedBy = [ ]` vs
> `wantedBy = [ "multi-user.target" ]` in the .nix file, then rebuild.

### Non-NixOS variant — `/etc/systemd/system/*.service`

```ini
# /etc/systemd/system/claude-heartbeat.service
[Unit]
Description=Claude Code Heartbeat

[Service]
Type=oneshot
ExecStart=/opt/claude-code/heartbeat.sh
Environment=PATH=/root/.local/bin:/usr/local/bin:/usr/bin:/bin
Environment=HOME=/root
```

```ini
# /etc/systemd/system/claude-heartbeat.timer
[Unit]
Description=Claude Code Heartbeat (every 5 min)

[Timer]
OnCalendar=*:0/5
Persistent=true

[Install]
WantedBy=timers.target
```

Activate:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now claude-heartbeat.timer
sudo systemctl enable --now claude-mm-review.timer
sudo systemctl enable --now claude-telegram.service
```

## Phase 7 — Telegram Bridge (multi-conversation)

The bridge is a long-running bash daemon that polls
`getUpdates`, dispatches slash-commands, and otherwise routes free
text into a `claude -p --resume <session-id>` invocation. Per-conversation
session ids are persisted under `~/.claude/channels/telegram/conversations/<name>/session-id.txt`
so each named conversation has independent memory.

Three things make it work:

1. **Session id capture.** After every `claude -p` call, the bridge
   reads the most recently modified file in `/root/.claude/sessions/`
   and saves it as the conversation's session id.
2. **Resume.** Next message in the same conversation is sent as
   `claude -p "$TEXT" --resume "$SESSION_ID"`.
3. **Cleanup.** Every ~30 polls the bridge prunes stale `claude -p`
   processes (older than 5 min) and `~/.claude/sessions/*` older than
   7 days. Without this you accumulate orphans on every fresh-started
   conversation.

The full reference script lives in
`/opt/claude-code/telegram-bridge.sh` on VPS3. Key contract excerpts:

```bash
# Each free-text message lands here
ACTIVE=$(get_active)
SESSION_ID=$(get_session_id "$ACTIVE")

if [[ -n "$SESSION_ID" ]]; then
    RESPONSE=$(cd /root/workspace && claude -p "$TEXT" --resume "$SESSION_ID" 2>/dev/null)
else
    RESPONSE=$(cd /root/workspace && claude -p "$TEXT" 2>/dev/null)
fi

# Persist the freshest session id
LATEST=$(ls -t /root/.claude/sessions/ 2>/dev/null | head -1)
[[ -n "$LATEST" ]] && save_session_id "$ACTIVE" "$LATEST"
```

Slash command surface (drop or extend as you like):

| Command            | Purpose                                                       |
|---|---|
| `/menu` `/help`    | Print the command list                                        |
| `/new [name]`      | Start a new conversation; defaults to `<mon><day>-<hhmm>`     |
| `/list`            | List saved conversations + which is active                    |
| `/switch <name>`   | Switch the active conversation                                |
| `/current`         | Show active conversation name + has-context flag              |
| `/delete <name>`   | Remove a conversation's session-id directory                  |
| `/status`          | Wallet, positions count, deployed capital, realized PnL       |
| `/positions`       | Per-market positions with cost basis                          |
| `/orders`          | Live open orders (CLOB)                                       |
| `/rewards`         | Reward earnings + qualification status                        |
| free text          | Forwarded into the active conversation                        |

The bridge needs `BOT_TOKEN` and `CHAT_ID`:

```bash
sudo mkdir -p /var/lib/osmoda/secrets
sudo install -m 600 /dev/stdin /var/lib/osmoda/secrets/telegram-bot-token <<<"<bot-token>"
# CHAT_ID is hardcoded in the bridge — edit the script to your numeric id
```

## Phase 8 — Skill Discovery

`~/.openclaw/skills/<name>/SKILL.md` files are still consumed by the
new runtime: when Claude Code reads the workspace's `CLAUDE.md`, you
mention the skills and their script paths there. The CLI itself does
NOT auto-load `.openclaw/skills/`, so the only changes needed:

1. Keep your existing `SKILL.md` files in place — they document the
   skill for human readers and for any `claude -p` invocation that asks
   "what skills are available".
2. Add a `## Skills` section to `/root/workspace/CLAUDE.md` that lists
   each skill, its script directory, config path, and the wallet/keys
   it uses (see Phase 3 example).
3. If you want a skill to be invokable as `/<name>` from the Claude
   Code REPL, add a frontmatter `name:` matching what you'll type, and
   place a copy under `~/.claude/skills/<name>/SKILL.md` (the CLI's
   global skills directory).

## Phase 9 — Cron jobs / one-shot triggers

If the source box used cron instead of systemd timers, the migration is
even simpler:

```cron
*/5  * * * * /opt/claude-code/heartbeat.sh
*/10 * * * * /opt/claude-code/rewards-status-10m.sh
0    */2 * * * /opt/claude-code/mm-review.sh
0    *  * * * /opt/claude-code/rewards-report.sh
```

`PATH` and `HOME` are already exported inside each wrapper, so cron
doesn't need an env block.

## Verification Checklist

Run all of these after `nixos-rebuild switch` (or `systemctl
daemon-reload && enable --now …`).

```bash
# 1. CLI is installed and authenticated
/root/.local/bin/claude --version
jq .claudeAiOauth.subscriptionType /root/.claude/.credentials.json
# Expected: 2.1.92 (or your pinned version) ; "max"

# 2. Smoke test with a trivial prompt — must NOT prompt for browser
HOME=/root /root/.local/bin/claude -p "say only the word: pong" 2>&1 | head
# Expected: pong

# 3. Auto-context is loading
HOME=/root /root/.local/bin/claude -p "what hostname am I on? answer in one line" 2>&1
# Expected: it correctly references CLAUDE.md content (your hostname)

# 4. Tool permissions — should run without prompting
HOME=/root /root/.local/bin/claude -p "run: echo hello && date -u" 2>&1
# Expected: hello + timestamp, no permission gate

# 5. Systemd units active
systemctl list-timers --all --no-pager | grep claude
systemctl status claude-telegram --no-pager | head -5

# 6. First wrapper run by the timer
journalctl -u claude-heartbeat --no-pager -n 20

# 7. Telegram round-trip
# Send "/menu" from your Telegram client; expect the menu reply.

# 8. Session is being persisted
ls -t /root/.claude/projects/-root-workspace/*.jsonl | head
# Expect freshly-created jsonl files after step 7.

# 9. Confirm OpenClaw legacy units are disabled and not fighting
systemctl status osmoda-gateway --no-pager | head -3
# Expected: inactive (dead) ; disabled
```

## Troubleshooting

| Symptom | Diagnosis | Fix |
|---|---|---|
| `claude` exits immediately with `please run /login` | Either missing `.credentials.json` or the OAuth has expired and refresh failed | Re-copy from a working host (Phase 2 Option B); verify `expiresAt` is in the future |
| `claude -p` hangs forever | Workspace dir has no `CLAUDE.md` and CLI is asking for trust dialog | `cd /root/workspace` first; OR pass `--print` (alias `-p`) which auto-skips the trust dialog |
| Wrapper logs `Reached max turns` | Default turn budget exceeded | Pass `--max-turns 5` (or higher) on the `claude -p` call |
| Telegram bridge replies blank | The wrapper captured stderr instead of stdout | Use `RESPONSE=$(claude -p "$P" 2>/dev/null)` (suppress stderr) |
| Cost still appearing on Anthropic API dashboard | `ANTHROPIC_API_KEY` is set somewhere | `grep -r ANTHROPIC_API_KEY /etc/ /root/ 2>/dev/null` and remove |
| NixOS rebuild fails: "unit already exists" | Old `osmoda-gateway` and new `claude-*` define the same unit | Make sure you set `systemd.services.osmoda-gateway.enable = false;` in `claude-code.nix` |
| `claude -p` works manually but fails under systemd | `PATH` or `HOME` not exported | Add both to the unit's `Environment=`; `HOME=/root` is mandatory or the CLI can't find `~/.claude/.credentials.json` |
| Two telegram bots replying to the same message | Forgot to disable `openclaw-gateway` on the source box | Stop + disable it; the new bridge takes over polling |
| Session memory leaks (disk fills with jsonl) | Bridge cleanup loop not running | Confirm the `CYCLE % 60 == 0` block runs; manually `find /root/.claude/sessions -mtime +7 -delete` |
| `claude` prompts for permission on every Bash call | Per-project settings missing | Phase 4: write `/root/workspace/.claude/settings.json` |

## Rate-limit Notes

The Max plan uses a token bucket — `default_claude_max_20x` (20× plan)
gives roughly 20× the per-5h quota of Pro. Concrete budget on a busy
trading box (VPS3 reference):

| Wrapper           | Cadence       | Avg tokens / call | Daily turns |
|---|---|---|---|
| `heartbeat.sh`    | every 5 min   | 0 (no LLM unless alert) | 0–10 |
| `mm-review.sh`    | every 2 h     | 8–15k             | 12 |
| `rewards-status`  | every 10 min  | 0 (pure node)     | 0 |
| Telegram free chat| user-driven   | varies            | 20–80 |
| Heartbeat alerts  | rare          | 1–3k              | 0–5 |

That's well inside the Max 20× budget. If you outgrow it, the same
wrappers also work with `claude -p --model haiku` to drop to a cheaper
tier per call.

## Rollback

Every step is reversible without touching the OpenClaw skill scripts:

```bash
# 1. Disable the new units
sudo systemctl disable --now claude-telegram \
    claude-heartbeat.timer claude-mm-review.timer \
    rewards-quoter.service 2>/dev/null

# 2. Re-enable the old gateway (NixOS: flip enable = true; rebuild)
sudo systemctl enable --now osmoda-gateway

# 3. (Optional) remove the runtime
sudo rm -rf /opt/claude-code /root/.claude /root/workspace
sudo rm /root/.local/bin/claude

# Skill scripts and state at ~/.openclaw/* are untouched and the
# original openclaw-gateway picks up where it left off.
```

## Reference Implementation

The working reference is on **VPS3 (clawdbot3 / 76.13.250.48)**.
Inspect the live files for the canonical shape:

```bash
ssh clawdbot3
sudo cat /etc/nixos/claude-code.nix
sudo cat /opt/claude-code/heartbeat.sh
sudo cat /opt/claude-code/mm-review.sh
sudo cat /opt/claude-code/telegram-bridge.sh
sudo cat /root/.claude/settings.json
sudo cat /root/workspace/CLAUDE.md
```

Versions known to work as of 2026-04-09:

- Claude Code: `2.1.92`
- Node.js: `v22.22.0` (NixOS nixpkgs)
- NixOS: `26.05`
- Anthropic plan: Max 20× (`default_claude_max_20x`)

## Summary — what changes vs what stays

| Layer | Source (OpenClaw) | Path A (osModa automated) | Path B (manual wrappers) |
|---|---|---|---|
| Billing | API credits per token | Max-plan OAuth OR Console API key | Max-plan quota (fixed) |
| Auth | `ANTHROPIC_API_KEY` env | `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY` | `~/.claude/.credentials.json` OAuth |
| Invocation | `openclaw run …` / plugin | osmoda-gateway → `claude -p` + MCP bridge | `claude -p "$PROMPT"` wrappers |
| Gateway | `osmoda-gateway` (OpenClaw) | `osmoda-gateway` (Claude Code SDK, port 18789) | `claude-telegram.service` (bridge) |
| Tools | osmoda-bridge plugin (91) | osmoda-mcp-bridge (91 MCP tools) | Built-in Bash/Read/Write/Edit |
| Periodic jobs | openclaw plugins / cron | osmoda-routines daemon | systemd timers calling shell wrappers |
| Skill scripts | `~/.openclaw/skills/*` | `/root/workspace/skills/*` | **unchanged** |
| Secrets | `/var/lib/osmoda/secrets/*` | **unchanged** | **unchanged** |
| Telegram | OpenClaw plugin | Gateway webhook handler | bash bridge → `claude -p --resume` |
| Session memory | OpenClaw conversations | Gateway in-memory sessions | `~/.claude/projects/-root-workspace/*.jsonl` |

The migration is intentionally non-destructive on the **skill** layer
(everything in `~/.openclaw/skills/` keeps working as-is) and aggressive
on the **runtime** layer (no more API credit billing, no more agentd
gateway). One Phase-1 install + a Phase-2 token copy + a single
nixos-rebuild flips the whole box.
