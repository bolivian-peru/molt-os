# Swarm Plan: Multi-Agent per Server via OpenClaw

## Architecture

```
1 Server (osModa instance)
  └── 1 OpenClaw Gateway process
        ├── Agent "osmoda" (default, Opus, full system access, web chat)
        ├── Agent "mobile" (Sonnet, Telegram/WhatsApp, concise)
        ├── Agent "marketing" (user-created, e.g. Sonnet)
        ├── Agent "scraper" (user-created, e.g. Haiku)
        └── ... (N agents, all in same process)
```

No Docker containers. No separate processes. No overhead.
OpenClaw handles routing, isolation, and session management natively.

Each agent is a config entry in `~/.openclaw/openclaw.json` with:
- **Workspace** (`~/.openclaw/workspace-<id>/`) — own SOUL.md, AGENTS.md, skills
- **Session store** (`~/.openclaw/agents/<id>/sessions`) — isolated chat history
- **Auth profile** (`~/.openclaw/agents/<id>/agent/`) — shares server's API key
- **Model** — Opus ($), Sonnet ($$), Haiku ($$$-cheap)
- **Tools** — can allow/deny specific tools per agent
- **Bindings** — route Telegram chats, WhatsApp contacts, or web sessions to specific agents

## What Already Works (MVP Today)

- Server spawns with 2 agents (osmoda + mobile)
- Telegram connection via dashboard
- Web chat to the agent
- 83 system tools, 17 skills
- All 9 Rust daemons running
- API key management from dashboard

## What To Build

### Phase 1: Create/Delete Agents from Dashboard

#### Backend (server.js)

1. **New heartbeat action: `create_agent`**
   - Payload: `{ id, name, model, personality, tools_profile }`
   - On server: writes new entry to `openclaw.json` agents.list[]
   - Creates workspace directory with SOUL.md (from personality) and AGENTS.md
   - Restarts gateway to pick up new agent
   - Reports completion back to spawn

2. **New heartbeat action: `delete_agent`**
   - Payload: `{ agent_id }`
   - On server: removes agent from `openclaw.json`
   - Optionally archives workspace
   - Restarts gateway
   - Reports completion

3. **New heartbeat action: `update_agent`**
   - Payload: `{ agent_id, model?, personality?, tools_profile? }`
   - Updates openclaw.json + workspace files
   - Restarts gateway

4. **Server state**: heartbeat already reports `agents[]` — extend to include:
   - agent id, name, model, status (running/stopped)
   - workspace path
   - session count
   - last active timestamp

#### Frontend (dashboard.html — server detail page)

1. **Agents section** on each server page:
   - List of agents with: name, model badge, status indicator
   - "osmoda" and "mobile" shown as system agents (not deletable)
   - User-created agents shown with edit/delete buttons

2. **"+ New Agent" button** → modal with:
   - Name (text input)
   - Model (dropdown: Opus / Sonnet / Haiku)
   - Personality (textarea — becomes SOUL.md)
   - Tools profile (dropdown: full / coding / messaging / minimal)
   - Optional: Telegram binding (route specific chat to this agent)

3. **Agent chat**: click agent → opens web chat to that specific agent
   (route via OpenClaw gateway with agent_id in session)

#### install.sh Changes

1. Heartbeat action handler for `create_agent`:
   ```bash
   create_agent)
     AGENT_ID=$(echo "$ACTION_JSON" | jq -r '.agent_id')
     AGENT_NAME=$(echo "$ACTION_JSON" | jq -r '.name')
     AGENT_MODEL=$(echo "$ACTION_JSON" | jq -r '.model')
     PERSONALITY=$(echo "$ACTION_JSON" | jq -r '.personality')

     # Create workspace
     WS_DIR="/root/.openclaw/workspace-${AGENT_ID}"
     mkdir -p "$WS_DIR"
     echo "$PERSONALITY" > "$WS_DIR/SOUL.md"

     # Add to openclaw.json via node
     node - "$AGENT_ID" "$AGENT_NAME" "$AGENT_MODEL" "$WS_DIR" <<'CREATEEOF'
     // Read openclaw.json, add agent to list, write back
     CREATEEOF

     # Restart gateway
     systemctl restart osmoda-gateway.service
   ```

2. Similar handlers for `delete_agent`, `update_agent`

### Phase 2: Agent Routing & Telegram Integration

1. **Per-agent Telegram binding**: route specific Telegram groups or DMs to specific agents
   - User creates Telegram group "Marketing"
   - Binds it to the "marketing" agent
   - Messages in that group go to marketing agent only

2. **Master Telegram chat**: the default mobile agent handles DMs
   - Can list all agents: "show my agents"
   - Can relay messages: "tell marketing agent to run the campaign"
   - Gets alerts from all agents

3. **Agent-to-agent communication**: OpenClaw supports `tools.agentToAgent`
   - Marketing agent can ask code agent to deploy something
   - Monitor agent can tell the main agent about issues

### Phase 3: Agent Templates (Marketplace)

Pre-built agent configurations users can one-click deploy:

1. **Marketing Agent** — Sonnet
   - SOUL: Expert digital marketer, manages campaigns, tracks metrics
   - Tools: web search, web fetch, exec (for scripts), file write
   - Use case: Run marketing campaigns, write content, post on socials

2. **Code Agent** — Opus
   - SOUL: Expert programmer, builds and deploys applications
   - Tools: full (all tools)
   - Use case: Build apps, fix bugs, deploy services

3. **Monitor Agent** — Haiku (cheap, runs often)
   - SOUL: Infrastructure watchdog, brief alerts only
   - Tools: system health, journal logs, service status
   - Heartbeat: every 5 min, checks services, alerts on Telegram
   - Use case: 24/7 monitoring with cheap model

4. **Scraper Agent** — Sonnet
   - SOUL: Data collection specialist
   - Tools: web search, web fetch, exec, file write
   - Use case: Scrape data, build datasets, run scheduled collections

5. **Customer Support Agent** — Sonnet
   - SOUL: Helpful support rep, knows product docs
   - Tools: web search, file read, messaging
   - Binding: WhatsApp business number
   - Use case: Answer customer questions 24/7

6. **Research Agent** — Opus
   - SOUL: Deep researcher, writes thorough reports
   - Tools: web search, web fetch, file write
   - Use case: Market research, competitive analysis, report writing

## Technical Notes

### OpenClaw Config Structure (what we write)

```json5
{
  agents: {
    list: [
      { id: "osmoda", default: true, name: "osModa", workspace: "...", model: "anthropic/claude-opus-4-6" },
      { id: "mobile", name: "osModa Mobile", workspace: "...", model: "anthropic/claude-sonnet-4-6" },
      // User-created agents added here:
      { id: "marketing", name: "Marketing", workspace: "...", model: "anthropic/claude-sonnet-4-6",
        tools: { profile: "full" } },
      { id: "monitor", name: "Monitor", workspace: "...", model: "anthropic/claude-haiku-4-5",
        heartbeat: { every: "5m", prompt: "Check system health..." } }
    ]
  },
  bindings: [
    { agentId: "mobile", match: { channel: "telegram" } },
    { agentId: "marketing", match: { channel: "telegram", peer: { kind: "group", id: "-1001234567" } } }
  ]
}
```

### Gateway Restart

After modifying `openclaw.json`, restart the gateway:
```bash
systemctl restart osmoda-gateway.service
```

OpenClaw supports hot-reload for most config changes, but agent list changes
require a restart to initialize new workspaces and session stores.

### Cost Implications

Each agent uses its own model and has its own conversations.
- Opus: ~$15/MTok input, $75/MTok output — for complex tasks
- Sonnet: ~$3/MTok input, $15/MTok output — good balance
- Haiku: ~$0.25/MTok input, $1.25/MTok output — cheap monitoring/simple tasks

Users should be aware that more agents = more API costs.
The server cost (osModa plan) stays the same regardless of agent count.

### Limits

- Max agents per server: reasonable cap at 10-20 (no hard technical limit)
- All agents share the same API key (per-agent keys possible but complex)
- All agents share the same 83 osModa tools (per-agent tool profiles can restrict)
- Memory (RAM) constraint: each agent maintains session state in memory
  - Solo plan (4GB): comfortable with 3-5 agents
  - Pro plan (8GB): 5-10 agents
  - Team plan (16GB): 10-15 agents
  - Scale plan (32GB): 15-20+ agents

## Implementation Order

1. **Phase 1a**: `create_agent` heartbeat action + install.sh handler
2. **Phase 1b**: Dashboard UI — agents list + create modal on server page
3. **Phase 1c**: `delete_agent` + `update_agent` actions
4. **Phase 2a**: Per-agent Telegram bindings from dashboard
5. **Phase 2b**: Master agent can list/manage sub-agents
6. **Phase 3**: Agent templates / marketplace
