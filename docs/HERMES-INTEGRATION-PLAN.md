# Hermes Agent Integration Plan for osModa

**Updated**: 2026-03-27 — Aligned with Hermes v0.4.0, current osModa state, and production reality.

---

## 1. Why This Matters

osModa's architecture doc promises "NixOS distribution with AI-native system management." The reality today:

- All spawned servers run install.sh on NixOS snapshots (just fixed)
- 10 Rust daemons provide 90 tools via osmoda-bridge → OpenClaw
- OpenClaw is the only runtime, locked to Anthropic models
- teachd has OBSERVE/LEARN loops but skillgen produces nothing usable yet
- Memory is flat (single FTS5 bucket)

Hermes v0.4.0 ships with:
- Official NixOS flake + module (native systemd + OCI container modes)
- 40+ built-in tools, native MCP client, 12 messaging platforms
- LLM-driven skill auto-creation (agent creates skills via tool calls)
- 3-tier memory (session, persistent, procedural)
- 15+ model providers (Anthropic, OpenRouter, Ollama, DeepSeek, etc.)
- OpenClaw migration: `hermes claw migrate`
- ACP (Agent Client Protocol) for editor/IDE integration

The integration strategy: **not Hermes OR OpenClaw, but Hermes AND OpenClaw** — same daemons, same tools, user picks their runtime.

---

## 2. Architecture: Dual Runtime

```
                     ┌─────────────────────────────┐
                     │         osmoda.nix           │
                     │  runtime = "openclaw"|"hermes"│
                     └──────┬──────────────┬────────┘
                            │              │
              ┌─────────────▼──┐     ┌─────▼──────────────┐
              │   OpenClaw      │     │   Hermes Agent      │
              │  (Node.js)      │     │   (Python)          │
              │ osmoda-bridge   │     │   MCP client        │
              │  90 tools       │     │   90 tools (MCP)    │
              │  Claude models  │     │   Any model         │
              │  Telegram, WA   │     │   12 platforms      │
              └───────┬─────────┘     └─────┬──────────────┘
                      │                     │
                      └─────────┬───────────┘
                                │
                      ┌─────────▼──────────┐
                      │  osmoda-mcp-bridge  │ ← NEW: MCP server
                      │  (stdio transport)  │    wrapping all daemon APIs
                      └─────────┬──────────┘
                                │
              ┌────────┬────────┼────────┬────────┬────────┐
              ▼        ▼        ▼        ▼        ▼        ▼
           agentd    keyd    watch   routines   mesh    teachd
           (.sock)  (.sock)  (.sock)  (.sock)  (.sock)  (.sock)
```

**Key insight**: Both runtimes talk to the same 10 Rust daemons. The difference is how:
- **OpenClaw**: osmoda-bridge (TypeScript plugin) calls Unix sockets directly
- **Hermes**: osmoda-mcp-bridge (new MCP server, Rust or TypeScript) exposes the same APIs as MCP tools. Hermes's native MCP client discovers and uses them.

Zero tool reimplementation. Add a tool to agentd → available to both runtimes.

---

## 3. The MCP Bridge (Critical New Component)

### Why MCP, not native Python tools

Hermes has a Python tool registry (`tools/registry.py`). We COULD port all 90 tools to Python. But:
- Hermes already has native MCP support (stdio + HTTP, auto-discovery, reconnection)
- MCP tools are language-agnostic — any future runtime also gets them
- The tool schemas already exist in osmoda-bridge's `registerTool()` calls

### Implementation: `osmoda-mcp-bridge`

A single MCP server (stdio transport) that exposes all daemon Unix socket APIs as MCP tools.

**Option A: TypeScript** (reuse osmoda-bridge code directly)
```
/opt/osmoda/bin/osmoda-mcp-bridge.js
  ← reads MCP JSON-RPC from stdin
  → writes MCP JSON-RPC to stdout
  → calls Unix sockets internally (same code as osmoda-bridge)
```

**Option B: Rust** (new crate, cleaner but more work)
```
crates/osmoda-mcp-bridge/
  src/main.rs        # MCP server over stdio
  src/tools.rs       # Tool definitions (JSON Schema)
  src/dispatch.rs    # Route tool calls to daemon sockets
```

**Recommendation**: Option A (TypeScript). Extract the tool handler functions from `packages/osmoda-bridge/index.ts` into a shared module. The MCP bridge imports them and wraps in MCP JSON-RPC. Same logic, different transport.

**Hermes config** (`~/.hermes/config.yaml`):
```yaml
mcp_servers:
  osmoda:
    command: "node"
    args: ["/opt/osmoda/bin/osmoda-mcp-bridge.js"]
    env:
      AGENTD_SOCKET: "/run/osmoda/agentd.sock"
      KEYD_SOCKET: "/run/osmoda/keyd.sock"
```

Hermes auto-discovers all 90 tools at startup via MCP `list_tools()`.

**Effort**: 1-2 weeks. Most code is reuse from osmoda-bridge.

---

## 4. NixOS Module: Runtime Selector

Hermes now has its own official NixOS module (`nix/nixosModules.nix` in the Hermes repo). We integrate it alongside our existing OpenClaw setup.

### Module additions to `osmoda.nix`:

```nix
osmoda.runtime = mkOption {
  type = types.enum [ "openclaw" "hermes" ];
  default = "openclaw";
  description = "Agent runtime. OpenClaw (Anthropic-optimized) or Hermes (model-agnostic).";
};

osmoda.hermes = {
  model = mkOption {
    type = types.str;
    default = "anthropic/claude-sonnet-4-6";
    description = "Default model for Hermes agent.";
  };
  environmentFile = mkOption {
    type = types.nullOr types.path;
    default = null;
    description = "Path to .env file with API keys.";
  };
};
```

### What changes based on runtime:

| Component | OpenClaw | Hermes |
|---|---|---|
| Gateway service | `osmoda-gateway.service` (OpenClaw binary) | `hermes-agent.service` (Hermes binary) |
| Tool bridge | osmoda-bridge (TypeScript plugin) | osmoda-mcp-bridge (MCP server) |
| Config format | `openclaw.json` | `config.yaml` |
| Skill format | SKILL.md (identical!) | SKILL.md (identical!) |
| Agent identity | AGENTS.md + SOUL.md | SOUL.md (same format) |
| Channels | Telegram, WhatsApp | Telegram, Discord, Slack, WhatsApp, Signal, Matrix, +6 more |
| Models | Anthropic only | 200+ via OpenRouter, Ollama, etc. |

### What stays the same regardless of runtime:

- All 10 Rust daemons (agentd, keyd, watch, routines, mesh, mcpd, teachd, voice, egress, app-restore)
- Unix socket APIs
- Hash-chained audit ledger
- SafeSwitch deployments
- NixOS declarative config
- Heartbeat to spawn.os.moda
- WS relay for dashboard chat

---

## 5. Skill System: Best of Both

### osModa's teachd approach (daemon-driven)
```
OBSERVE (30s) → LEARN (5m) → SKILLGEN (6h) → SKILL.md
  Automatic, no LLM calls needed for detection
  Pattern matching + frequency analysis
  But: never actually produced a usable skill in production
```

### Hermes approach (LLM-driven)
```
Agent completes complex task → Agent decides to save skill → skill_manage(action="create")
  Uses the LLM's judgment about what's worth remembering
  But: costs API calls, quality depends on model
```

### Combined approach for osModa:

**Keep both.** teachd detects patterns automatically (free, no API calls). The agent reviews teachd's candidates and either promotes them to SKILL.md or dismisses them. This uses teachd's existing `teach_skill_candidates` + `teach_skill_generate` tools.

Additionally, both runtimes can create skills directly:
- OpenClaw: via `teach_skill_generate` tool call
- Hermes: via `skill_manage` (built-in) OR `teach_skill_generate` (via MCP)

All skills go to `skills/auto/` and use the same SKILL.md format. Portable between runtimes.

---

## 6. Spawn.os.moda Integration

### 6a. Runtime selector in spawn flow

When user creates a new server:
```
[Plan Card] → [Runtime: OpenClaw / Hermes] → [Model: Claude Opus / Sonnet / DeepSeek / ...] → [Deploy]
```

- `POST /api/v1/spawn/:planId` accepts `runtime` parameter
- `POST /api/dashboard/deploy` accepts `runtime` + `model` parameters
- Cloud-init passes `--runtime hermes` or `--runtime openclaw` to install.sh
- install.sh branches: install OpenClaw OR install Hermes + osmoda-mcp-bridge

### 6b. Dashboard chat adaptation

The WS relay currently bridges Browser ↔ OpenClaw WebSocket.

For Hermes:
- Hermes exposes an OpenAI-compatible API at `localhost:8642`
- The WS relay can POST to `http://localhost:8642/v1/chat/completions` with streaming
- Or use Hermes's ACP protocol (JSON-RPC over stdio) for richer interaction

The simplest path: WS relay detects which runtime is active and uses the right protocol.

### 6c. Model picker

Dashboard Settings tab adds model dropdown:
- Fetches available models from OpenRouter API (cached)
- Groups by provider (Anthropic, Meta, Mistral, DeepSeek, etc.)
- User selects → pending action → heartbeat delivers → restart gateway

---

## 7. install.sh Changes

### New flag: `--runtime`

```bash
# Existing
curl -fsSL .../install.sh | bash -s -- --skip-nixos --order-id '...'

# New
curl -fsSL .../install.sh | bash -s -- --runtime hermes --order-id '...'
curl -fsSL .../install.sh | bash -s -- --runtime openclaw --order-id '...'  # default
```

### Hermes install path (when `--runtime hermes`):

1. Skip OpenClaw installation
2. Install Hermes:
   ```bash
   # Hermes has a Nix flake — use it
   nix profile install github:nousresearch/hermes-agent
   ```
3. Generate `~/.hermes/config.yaml` from install args (model, API key)
4. Generate `~/.hermes/SOUL.md` from templates (reuse existing SOUL.md)
5. Copy skills from `skills/` to `~/.hermes/skills/`
6. Build + install osmoda-mcp-bridge
7. Configure MCP server in `config.yaml` pointing to osmoda-mcp-bridge
8. Create `hermes-agent.service` systemd unit
9. Start Hermes gateway

### Shared steps (both runtimes):

- Rust daemon build (agentd, keyd, watch, routines, mesh, etc.)
- Systemd service creation for all daemons
- Heartbeat setup
- WS relay setup (adapted per runtime)
- Device identity generation

---

## 8. Implementation Plan — 8-Hour Sprint Blocks

Each phase is scoped for an 8-hour focused sprint. Phases are sequential — each builds on the previous. Every phase has an exact prompt you can paste into a Claude Code session to execute it.

---

### Phase 1: MCP Bridge Foundation (8 hours)

**Goal**: A working MCP server that exposes all 90 osModa tools over stdio.

**Pre-research done**:
- osmoda-bridge has 90 tools in `packages/osmoda-bridge/index.ts`
- Each tool has: name, description, JSON Schema params, handler function
- Handler functions call daemon Unix sockets via `agentdRequest()`, `keydRequest()`, etc.
- MCP protocol: JSON-RPC over stdio, `tools/list` returns schemas, `tools/call` executes
- Hermes discovers MCP tools at startup via `list_tools()`

**Files to create**:
```
packages/osmoda-mcp-bridge/
  package.json           # name: osmoda-mcp-bridge, deps: @modelcontextprotocol/sdk
  index.ts               # MCP server: reads stdio, routes tool calls to daemon sockets
  tools.ts               # Tool registry: 90 tool definitions (name, schema, handler)
  daemon-clients.ts      # Shared Unix socket HTTP clients (copied from osmoda-bridge)
```

**Exact approach**:
1. `npm init` the new package with `@modelcontextprotocol/sdk` dependency
2. Extract all `agentdRequest`, `keydRequest`, `watchRequest`, `routinesRequest`, `meshRequest`, `mcpdRequest`, `teachdRequest` HTTP-over-Unix-socket clients into `daemon-clients.ts`
3. Extract all 90 tool definitions (name + JSON Schema + handler) from `index.ts` into `tools.ts` as a flat array
4. In `index.ts`, create MCP `Server` instance, register all 90 tools via `server.setRequestHandler(ListToolsRequestSchema, ...)` and `server.setRequestHandler(CallToolRequestSchema, ...)`
5. Connect server to stdio transport: `new StdioServerTransport()`
6. Test locally: `echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | node packages/osmoda-mcp-bridge/index.ts`

**Sprint prompt** (paste into Claude Code):
```
Read packages/osmoda-bridge/index.ts fully. Extract all 90 tool registrations into a new MCP server package at packages/osmoda-mcp-bridge/. The MCP server must:
1. Use @modelcontextprotocol/sdk (npm package)
2. Run over stdio transport (stdin/stdout JSON-RPC)
3. Export all 90 tools with identical names, descriptions, and JSON Schema params
4. Route tool calls to the same daemon Unix sockets as osmoda-bridge
5. Share the HTTP-over-Unix-socket client code (agentdRequest, keydRequest, etc.)
Test by piping a tools/list request through stdin and verifying all 90 tools appear.
Do NOT modify osmoda-bridge — create a new standalone package.
```

**Verification**: `cat test-list.json | node packages/osmoda-mcp-bridge/index.ts` returns 90 tools.

---

### Phase 2: Hermes + MCP Bridge End-to-End (8 hours)

**Goal**: Hermes running locally, calling osModa tools via the MCP bridge, against a real server.

**Pre-research done**:
- Hermes installs via `pip install hermes-agent` or `nix profile install github:nousresearch/hermes-agent`
- Config at `~/.hermes/config.yaml` with `mcp_servers:` section
- Hermes auto-discovers MCP tools on startup
- Hermes has `hermes claw migrate` to import OpenClaw config
- Hermes gateway mode: `hermes gateway` runs as a daemon

**Steps**:
1. Install Hermes locally (pip or nix)
2. Configure `~/.hermes/config.yaml`:
   ```yaml
   model:
     default: "anthropic/claude-sonnet-4-6"
   mcp_servers:
     osmoda:
       command: "node"
       args: ["packages/osmoda-mcp-bridge/index.ts"]
       env:
         AGENTD_SOCKET: "/run/osmoda/agentd.sock"
   ```
3. SSH tunnel to a real osModa server: `ssh -L /tmp/agentd.sock:/run/osmoda/agentd.sock root@168.119.157.243`
4. Point MCP bridge at the tunneled socket
5. Run `hermes` and test: "Check system health" → should call `system_health` via MCP → return real data
6. Test 5 critical tools: `system_health`, `shell_exec`, `file_read`, `service_status`, `journal_logs`

**Sprint prompt**:
```
I have a working osmoda-mcp-bridge at packages/osmoda-mcp-bridge/. Now:
1. Install Hermes agent (pip install hermes-agent or use nix)
2. Create a test config at ~/.hermes/config.yaml that connects to osmoda-mcp-bridge
3. I have an SSH tunnel to a live osModa server at /tmp/agentd.sock
4. Configure the MCP bridge to use that socket
5. Test that Hermes can call system_health, shell_exec, file_read, service_status, journal_logs
6. Document any issues with tool parameter mapping between Hermes and our schemas
```

**Verification**: Hermes chat session successfully calls 5 osModa tools and returns real system data.

---

### Phase 3: install.sh `--runtime hermes` (8 hours)

**Goal**: Fresh NixOS server installs with Hermes instead of OpenClaw.

**Pre-research done**:
- Hermes has official Nix flake at `github:nousresearch/hermes-agent`
- `nix profile install github:nousresearch/hermes-agent` installs `hermes` binary
- Hermes config: `~/.hermes/config.yaml` (model, mcp_servers, etc.)
- Hermes identity: `~/.hermes/SOUL.md` (same format as OpenClaw's)
- Hermes service: `hermes gateway` as systemd ExecStart

**Changes to install.sh**:
1. New flag: `--runtime hermes|openclaw` (default: openclaw)
2. When `--runtime hermes`:
   - Skip OpenClaw npm install (Step 5)
   - Skip osmoda-bridge setup (Step 6)
   - Instead: `nix profile install github:nousresearch/hermes-agent`
   - Build osmoda-mcp-bridge: `cd /opt/osmoda/packages/osmoda-mcp-bridge && npm install`
   - Generate `~/.hermes/config.yaml` with model + mcp_servers pointing to osmoda-mcp-bridge
   - Copy `templates/SOUL.md` to `~/.hermes/SOUL.md`
   - Copy `skills/` to `~/.hermes/skills/`
   - Create `hermes-agent.service` systemd unit: `ExecStart=hermes gateway`
3. Shared steps (both runtimes): daemon build, heartbeat, ws-relay, device identity

**Sprint prompt**:
```
Read scripts/install.sh. Add a --runtime flag (hermes|openclaw, default openclaw).
When --runtime hermes:
- Skip Steps 5-6 (OpenClaw + osmoda-bridge)
- Install Hermes via: nix profile install github:nousresearch/hermes-agent
- Build the MCP bridge: cd /opt/osmoda/packages/osmoda-mcp-bridge && npm install
- Generate ~/.hermes/config.yaml with:
  - model from --provider/--api-key args
  - mcp_servers.osmoda pointing to /opt/osmoda/packages/osmoda-mcp-bridge/index.ts
- Copy templates/SOUL.md to ~/.hermes/SOUL.md
- Copy skills/ to ~/.hermes/skills/
- Create /run/systemd/system/hermes-agent.service (or /etc/systemd/system/ on Ubuntu)
- WS relay adaptation: detect hermes runtime and use HTTP API at localhost:8642
Keep all daemon installation identical for both runtimes.
Test on the NixOS snapshot (image 370676004) on Hetzner.
```

**Verification**: `curl | bash -s -- --runtime hermes --skip-nixos` on NixOS snapshot → Hermes running, MCP bridge connected, 90 tools available.

---

### Phase 4: WS Relay + Dashboard Chat for Hermes (8 hours)

**Goal**: Dashboard chat works with Hermes runtime (browser ↔ spawn ↔ Hermes).

**Pre-research done**:
- Current ws-relay connects to OpenClaw's WebSocket at `ws://127.0.0.1:18789`
- Hermes exposes OpenAI-compatible API at `http://127.0.0.1:8642`
- Hermes API: `POST /v1/chat/completions` with streaming (SSE)
- Dashboard chat endpoint at `POST /api/dashboard/servers/:id/chat` uses `room.agent`

**Changes**:
1. WS relay (`osmoda-ws-relay.js`): detect runtime by checking which port responds
   - Port 18789 open → OpenClaw mode (existing WebSocket protocol)
   - Port 8642 open → Hermes mode (HTTP streaming)
2. Hermes mode in ws-relay:
   - Receive `{ type: "chat", text: "..." }` from spawn
   - POST to `http://127.0.0.1:8642/v1/chat/completions` with `stream: true`
   - Parse SSE chunks → forward as `{ type: "event", event: "agent", payload: { stream: "assistant", data: { delta: "..." } } }`
   - On `[DONE]` → forward lifecycle end event
3. The spawn server (server.js) needs zero changes — it talks to `room.agent` (ws-relay) which handles the protocol difference

**Sprint prompt**:
```
Read /opt/osmoda/bin/osmoda-ws-relay.js on server 168.119.157.243 (SSH via spawn server).
Also read the Hermes OpenAI-compatible API docs at http://localhost:8642.
Modify osmoda-ws-relay.js to support both runtimes:
1. On startup, probe port 18789 (OpenClaw) and 8642 (Hermes)
2. If OpenClaw: use existing WebSocket protocol (unchanged)
3. If Hermes: when receiving { type: "chat", text } from spawn, POST to localhost:8642/v1/chat/completions with streaming, convert SSE chunks to the same event format spawn expects
4. The spawn server (server.js) must not need any changes
Test by switching a test server from OpenClaw to Hermes and verifying dashboard chat works.
```

**Verification**: Send message in dashboard chat → Hermes processes it → response appears in browser.

---

### Phase 5: Spawn Runtime Selector (8 hours)

**Goal**: Users choose runtime when spawning on spawn.os.moda.

**Changes**:
1. **Plan cards** (`index.html`): Add runtime toggle (OpenClaw default / Hermes)
2. **Spawn API** (`server.js`):
   - `POST /api/dashboard/deploy` accepts `runtime` field
   - `POST /api/v1/spawn/:planId` accepts `runtime` field
   - Cloud-init passes `--runtime hermes` or `--runtime openclaw` to install.sh
   - Store `runtime` in order record
3. **Dashboard Settings**: Show which runtime is active, option to switch (triggers rebuild)
4. **Model picker**: When Hermes selected, show expanded model dropdown (OpenRouter models)
5. **Heartbeat**: Report which runtime is active so dashboard shows correct status

**Sprint prompt**:
```
Modify the spawn.os.moda dashboard to support runtime selection:
1. index.html plan cards: add a small toggle "Runtime: OpenClaw | Hermes" (default OpenClaw)
2. server.js deploy/spawn endpoints: accept runtime parameter, pass --runtime to cloud-init
3. server.js order schema: add runtime field
4. dashboard.html Settings tab: show active runtime
5. dashboard.html Overview: show runtime badge next to agent info
6. When Hermes is selected in spawn flow, show model dropdown with popular options:
   Claude Opus, Claude Sonnet, DeepSeek V3, Llama 3.3, Mistral Large, Qwen3
Existing servers stay unchanged. Only affects new spawns.
```

**Verification**: Spawn a new server with Hermes runtime → install completes → dashboard shows "Hermes" badge → chat works.

---

### Phase 6: Skill Convergence + Testing (8 hours)

**Goal**: Skills work across both runtimes. Full integration test.

**Changes**:
1. Copy all `skills/` to Hermes's skill directory during install
2. teachd's `teach_skill_generate` writes to `skills/auto/` — accessible by both runtimes
3. Hermes's built-in `skill_manage` also writes to `~/.hermes/skills/` — sync with `skills/auto/`
4. Add a `skill_sync` routine that copies between skill directories
5. Integration test script: spawn both runtime types, verify all daemons, test chat, test 10 tools

**Sprint prompt**:
```
1. Ensure install.sh copies all skills/ to both:
   - /root/.openclaw/workspace-osmoda/skills/ (OpenClaw)
   - /root/.hermes/skills/ (Hermes)
2. Add a skill sync mechanism: when teachd generates a skill in skills/auto/,
   also symlink or copy it to the active runtime's skill directory
3. Write an integration test script at scripts/test-integration.sh that:
   - Creates two Hetzner CX23 servers from NixOS snapshot 370676004
   - Installs one with --runtime openclaw, one with --runtime hermes
   - Waits for both to send heartbeat
   - Tests SSH, 5 tools via dashboard chat, file browser
   - Verifies all 10 daemons are running on both
   - Cleans up (deletes servers)
4. Run the test and fix any issues found
```

**Verification**: Both runtimes pass the integration test. Skills visible in both.

---

## 9. What We Get

| Capability | Today | After Integration |
|---|---|---|
| Agent runtimes | OpenClaw only | OpenClaw OR Hermes |
| Models | Anthropic only (Claude) | 200+ models (any provider) |
| Local models | No | Yes (Ollama, llama.cpp) |
| Messaging | Telegram, WhatsApp | +Discord, Slack, Signal, Matrix, +6 |
| Skill creation | teachd SKILLGEN (broken) | teachd + LLM-driven (both) |
| Memory | Flat FTS5 | 3-tier (session/persistent/skill) |
| MCP | osmoda-mcpd (daemon manager) | + osmoda-mcp-bridge (tool server) |
| IDE integration | None | ACP (Hermes) |
| Tool count | 90 | 90 (same tools, both runtimes) |
| NixOS native | Yes | Yes (both have NixOS modules) |
| Migration | N/A | `hermes claw migrate` (official) |

### The pitch

**osModa becomes the first NixOS distribution where users choose their AI brain.**

Want Claude with deep Anthropic integration? → OpenClaw.
Want model freedom with 200+ options? → Hermes.
Want local models with zero cloud dependency? → Hermes + Ollama.

Both runtimes manage the same NixOS system through the same 10 daemons, same audit ledger, same SafeSwitch, same skills. Switch runtimes without losing your system's learned knowledge.

---

## 10. Risks and Mitigations

| Risk | Mitigation |
|---|---|
| MCP bridge adds latency | Unix socket calls are <1ms. Benchmark before optimizing. |
| Hermes Python packaging on NixOS | Hermes has an official Nix flake with uv2nix. Use it directly. |
| Two runtimes = double testing | Share the daemon test suite. Only test the bridge layer per-runtime. |
| Hermes updates break MCP bridge | Pin Hermes to release tags. MCP protocol is stable. |
| Users confused by choice | Default to OpenClaw. Show Hermes as "Advanced" option with clear comparison. |
| teachd + Hermes skill systems conflict | teachd feeds candidates → runtime creates skills. No overlap. |
| Dashboard chat needs two protocols | WS relay detects runtime and adapts. Abstraction cost is low. |

---

## Appendix: File Changes Summary

| File | Change |
|---|---|
| `packages/osmoda-mcp-bridge/` | NEW: MCP server wrapping daemon APIs |
| `scripts/install.sh` | Add `--runtime hermes` path |
| `nix/modules/osmoda.nix` | Add `osmoda.runtime`, `osmoda.hermes` options |
| `flake.nix` | Import Hermes flake as input |
| `templates/agents/hermes/SOUL.md` | Hermes-adapted agent identity |
| `apps/spawn/server.js` | Runtime selector in spawn + rebuild |
| `apps/spawn/public/dashboard.html` | Runtime picker UI, model dropdown |
| `apps/spawn/public/index.html` | Runtime toggle on plan cards |
