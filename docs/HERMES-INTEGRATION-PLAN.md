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

## 8. Implementation Priority

### Phase 1: MCP Bridge (Week 1-2)
Build `osmoda-mcp-bridge.js` — extract tool handlers from osmoda-bridge, wrap in MCP stdio server. Test with Hermes locally.

**Deliverable**: `hermes` can call all 90 osModa tools via MCP.

### Phase 2: install.sh `--runtime hermes` (Week 3)
Add Hermes install path to install.sh. Test full install on NixOS snapshot.

**Deliverable**: `curl | bash -s -- --runtime hermes` produces a working Hermes + osModa server.

### Phase 3: NixOS Module (Week 4)
Add `osmoda.runtime` option to `osmoda.nix`. Import Hermes's flake. Generate configs.

**Deliverable**: `services.osmoda.runtime = "hermes"` works declaratively.

### Phase 4: Spawn Integration (Week 5-6)
Runtime selector in spawn flow. Dashboard chat adaptation. Model picker.

**Deliverable**: Users choose runtime + model when spawning on spawn.os.moda.

### Phase 5: Skill Convergence (Week 7-8)
Connect teachd's skill candidates to both runtimes. Shared `skills/auto/` directory.

**Deliverable**: Skills are portable between runtimes. teachd feeds both.

### Phase 6: Memory Upgrade (Week 9-10)
3-tier memory: session (in-memory), persistent (ZVEC + FTS5), procedural (skills).

**Deliverable**: Memory organized by purpose, better recall quality.

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
