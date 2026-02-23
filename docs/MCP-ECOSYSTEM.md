# osModa MCP Ecosystem

## The core is done. This is the expansion layer.

Last updated: 2026-02-23

The eight daemons are built. 54 tools are registered. The OS works. What comes next is not more custom tools — it's **MCP as the integration layer** that makes every capability in the MCP ecosystem available to your AI without writing another line of bridge code.

---

## What MCP gives us

OpenClaw already speaks MCP natively. Any MCP server becomes a set of tools available to the AI — no custom TypeScript, no new bridge code, no new Rust daemon. This changes the expansion model completely:

**Before (what we've been doing):**
```
New capability → write Rust daemon → write TypeScript bridge tool → register in osmoda-bridge → done
Time: days
```

**After (MCP ecosystem model):**
```
New capability → find/run MCP server → add to osmoda.nix → done
Time: minutes
```

osmoda-bridge stays exactly what it is: the OS tools layer (system, ledger, memory, wallets, deployments, scheduling, mesh). Everything else comes via managed MCP servers.

---

## Architecture

```
OpenClaw Gateway (:18789)
  │
  ├── osmoda-bridge (Kind: "tools") ← THE OS
  │     54 tools: agentd, keyd, watch, routines, mesh, voice
  │     Unix sockets, root access, NixOS mutations
  │
  └── MCP Servers (managed by osmoda-mcpd)
        ├── scrapling-mcp      → adaptive web scraping
        ├── postgres-mcp       → database access
        ├── github-mcp         → repos, issues, PRs
        ├── filesystem-mcp     → enhanced file ops
        ├── slack-mcp          → notifications
        └── any-mcp-server     → whatever you need
              │
              └── All outbound traffic → osmoda-egress (domain allowlist)
```

The OS tools are always there. MCP servers are opt-in capabilities. The AI sees all of them as tools in the same namespace.

---

## osmoda-mcpd — MCP Server Manager

A small new daemon that manages the lifecycle of configured MCP servers: starts them, restarts on failure, monitors health, routes all tool calls through the egress proxy.

**NixOS config:**

```nix
services.osmoda.mcp = {
  enable = true;
  socketPath = "/run/osmoda/mcpd.sock";

  servers = {
    # Web scraping — egress-gated, anti-bot bypass
    scrapling = {
      enable = true;
      command = "${pkgs.scrapling-mcp}/bin/scrapling-mcp";
      transport = "stdio";
      allowedDomains = [ "*.github.com" "news.ycombinator.com" ];  # egress
    };

    # Database access (local postgres)
    postgres = {
      enable = true;
      command = "${pkgs.nodejs}/bin/npx";
      args = [ "@modelcontextprotocol/server-postgres" ];
      env.DATABASE_URL = "postgresql:///mydb";
      transport = "stdio";
      # No egress needed — local socket
    };

    # GitHub — egress to api.github.com
    github = {
      enable = true;
      command = "${pkgs.nodejs}/bin/npx";
      args = [ "@modelcontextprotocol/server-github" ];
      secretFile = "/var/lib/osmoda/secrets/github-token";
      allowedDomains = [ "api.github.com" ];
    };

    # Custom local MCP server
    my-tool = {
      enable = true;
      command = "/usr/local/bin/my-mcp-server";
      transport = "stdio";
    };
  };
};
```

**Or tell the AI:**
> "Add a Postgres MCP server for my local database"
> AI writes to NixOS config, runs nixos-rebuild switch

---

## What osmoda-mcpd does (small Rust daemon)

osmoda-mcpd is not a proxy. It's a process manager + config generator:

1. **Reads** `services.osmoda.mcp.servers` from NixOS config
2. **Starts** each MCP server as a managed child process (stdio transport)
3. **Generates** OpenClaw config entries for each server
4. **Monitors** health — restarts crashed servers with backoff
5. **Routes** each server's outbound traffic through osmoda-egress (if `allowedDomains` set)
6. **Logs** MCP tool calls to agentd ledger (`mcp.tool.call`, `mcp.tool.result`)
7. **Reports** via `/health` on its Unix socket

**OpenClaw config generated:**

```json
{
  "mcp": {
    "servers": {
      "scrapling": {
        "command": "/nix/store/.../scrapling-mcp",
        "transport": "stdio",
        "env": { "HTTP_PROXY": "http://localhost:18801" }
      },
      "postgres": {
        "command": "npx",
        "args": ["@modelcontextprotocol/server-postgres"],
        "env": { "DATABASE_URL": "postgresql:///mydb" }
      }
    }
  }
}
```

OpenClaw reads this config and connects to each server. The AI gets all their tools alongside the OS tools.

---

## Tier 1 MCP servers to support out-of-box

These are the highest-value, most commonly needed:

| Server | Package | What it gives the AI |
|--------|---------|----------------------|
| `scrapling` | `scrapling-mcp` | Web scraping with anti-bot bypass |
| `filesystem` | `@modelcontextprotocol/server-filesystem` | Enhanced file ops beyond shell_exec |
| `postgres` | `@modelcontextprotocol/server-postgres` | SQL queries on local databases |
| `sqlite` | `@modelcontextprotocol/server-sqlite` | SQLite databases |
| `github` | `@modelcontextprotocol/server-github` | Repos, issues, PRs, code search |
| `fetch` | `@modelcontextprotocol/server-fetch` | Simple HTTP fetching (cheaper than scrapling) |
| `slack` | `@modelcontextprotocol/server-slack` | Read/write Slack |

Each gets a NixOS module option under `services.osmoda.mcp.servers.<name>`.

---

## Security model for MCP servers

MCP servers run in different trust contexts depending on what they access:

| Server type | Trust ring | Network | How |
|-------------|-----------|---------|-----|
| Local-only (postgres, sqlite, filesystem) | Ring 1 | None | `PrivateNetwork=true` in systemd |
| Web-fetching (scrapling, fetch, github) | Ring 1 | Via egress | `allowedDomains` → egress allowlist |
| Custom user servers | Ring 2 | Restricted | bubblewrap + egress |

Every MCP tool call is logged to the agentd ledger. The AI can explain every tool call it made, to what server, with what arguments.

---

## Memory: PageIndex verdict + forward path

### PageIndex — not suitable

PageIndex uses tree-structured indexing (interesting approach, no vector DB needed). The problem: **cloud-only**. Enterprise on-prem requires a contract. For osModa's "no third party in the data path" principle, this is a blocker. The API sends your documents to their servers.

### What we have now

FTS5 full-text search is live in agentd. BM25-ranked keyword search over all ledger events. Memory recall works. This handles 80% of practical cases — finding past diagnoses, user preferences, recent events.

### The remaining 20%: semantic search

FTS5 misses semantic queries like "find times the server was struggling" when the logs say "high load", "cpu spike", "process limit". Vector search catches these.

**Best fit for osModa: `fastembed-rs` + `usearch`**

| Crate | What it does | Size |
|-------|-------------|------|
| `fastembed` | Pure Rust embeddings, no Python, ONNX runtime | ~22MB model |
| `usearch` | In-process vector search, C++ core, Rust bindings | ~2MB binary |

Both are zero-dependency additions to agentd (no Python sidecar, no separate daemon, no 500MB model). The embedding model (all-MiniLM-L6-v2 quantized) is small enough to ship in the NixOS closure.

**What changes:**
- agentd gets two new deps: `fastembed` + `usearch`
- On first boot, model downloads to `/var/lib/osmoda/memory/model/` (22MB)
- `memory/ingest` starts embedding events in background (non-blocking)
- `memory/recall` runs FTS5 first, then usearch, RRF-merges results
- FTS5 remains the fallback if model not loaded yet

**ZVEC verdict:** Overengineered for this use case. usearch at 22MB vs ZVEC's 500MB+ infrastructure. Same quality at 1/20th the weight.

**PageIndex verdict:** Cloud API, data leaves the machine. Not compatible with osModa's principles.

**Implementation:** Sprint 2-3. ~300 LOC in agentd, no new daemons.

---

## Recommended new docs to write

1. **`docs/MCP-SERVERS.md`** — User guide: "How to add a new MCP server" (the `openclaw mcp add` flow vs NixOS config flow)
2. **`docs/MEMORY.md`** — How memory works end to end (replaces the planning doc with live reality)
3. See also: **`docs/X402.md`** — OS-native micropayments
