# AGENTOS — MASTER PLAN
## OpenClaw IS the Operating System. Not running on it. IS it.

---

## 0. THE PHILOSOPHY (Read This First)

**The fundamental insight everyone else gets wrong:**

Other "AI OS" projects put an agent *on top of* Linux. The agent asks permission.
The agent is sandboxed. The agent is a guest.

**We do the opposite.**

OpenClaw is the **master**. It has root. It sees every process, every file, every
network connection, every syscall log, every kernel parameter. It doesn't "ask"
to install packages — it owns `configuration.nix`. It doesn't "request" network
access — it controls `nftables`. It doesn't "monitor" services — it IS the service
manager's brain.

The entire NixOS system is OpenClaw's body. Every API running on the system, every
daemon, every cron job, every socket — OpenClaw can see it, control it, reconfigure
it, restart it, kill it.

**The sandbox is not for OpenClaw. The sandbox is for untrusted third-party tools
and apps that OpenClaw executes on behalf of the user.** OpenClaw itself runs with
full system access because it IS the system.

```
USER ←→ OpenClaw (THE OS) ←→ Linux Kernel
              ↓
         [sandboxed zone]
         third-party tools
         untrusted skills
         user scripts
```

**What "AI-native" actually means here:**

- "Install VS Code" → OpenClaw edits `configuration.nix`, validates, rebuilds. Done.
- "Why is my disk full?" → OpenClaw reads `du`, `df`, `journalctl`, `nix-store --gc --print-dead`, diagnoses, proposes cleanup, executes on approval.
- "Set up a Postgres database" → OpenClaw enables the NixOS PostgreSQL module, creates user, configures firewall, writes backup cron, rebuilds.
- "What's using my GPU?" → OpenClaw reads `/proc`, nvidia-smi, lsof, correlates with running sessions.
- "Deploy my app" → OpenClaw reads Dockerfile/flake, builds, configures systemd service, sets up reverse proxy, enables.
- "Something is wrong with networking" → OpenClaw reads `ip`, `ss`, `journalctl -u NetworkManager`, `nft list ruleset`, `resolvectl`, diagnoses.

Every API on the system is OpenClaw's API. Every running service is OpenClaw's service.
Every config file is OpenClaw's config file. **It inherits everything.**

---

## 1. ARCHITECTURE: THREE TRUST RINGS

```
╔══════════════════════════════════════════════════════════════════╗
║                    RING 0: OPENCLAW (GOD MODE)                  ║
║                                                                  ║
║  Full system access. Runs as root-equivalent.                    ║
║  Sees: all files, all processes, all network, all kernel params  ║
║  Controls: nixos-rebuild, systemctl, nftables, users, secrets    ║
║  Owns: /etc/nixos, /var/lib/agentos, all NixOS configuration    ║
║                                                                  ║
║  ┌──────────────────────────────────────────────────────────┐    ║
║  │ OpenClaw Gateway (WS :18789)                             │    ║
║  │ Pi Agent Runtime (RPC)                                   │    ║
║  │ agentd (Rust daemon — kernel bridge)                     │    ║
║  │ System Skills (full OS access)                           │    ║
║  │ Ledger (append-only audit of everything)                 │    ║
║  └──────────────────────────────────────────────────────────┘    ║
╠══════════════════════════════════════════════════════════════════╣
║                    RING 1: APPROVED APPS                        ║
║                                                                  ║
║  OpenClaw-native apps (openclawPlugin). Reviewed and installed.  ║
║  Get: declared capabilities (network, fs paths, tools)           ║
║  Don't get: root, arbitrary fs, kernel params                    ║
║  Sandbox: bubblewrap + systemd transient units                   ║
║  Network: only through egress proxy with allowlist               ║
║                                                                  ║
╠══════════════════════════════════════════════════════════════════╣
║                    RING 2: UNTRUSTED EXECUTION                  ║
║                                                                  ║
║  User scripts, pip packages, npm installs, random binaries.      ║
║  Get: working dir + /tmp. Nothing else.                          ║
║  Sandbox: max isolation bubblewrap (no network, no /home)        ║
║  OpenClaw watches, logs, can kill.                               ║
║                                                                  ║
╚══════════════════════════════════════════════════════════════════╝
```

### Why Three Rings?

OpenClaw needs full access because it IS the interface to the system.
But when OpenClaw installs a third-party skill from ClawHub and runs
its tool, that tool should NOT get root access. OpenClaw decides what
each tool gets, grants it via capability tokens, and revokes after
execution.

**OpenClaw = the kernel's brain. Sandbox = the kernel's immune system.**

---

## 2. SYSTEM COMPONENTS (What Gets Built)

### 2.1 agentd — The Kernel Bridge (Rust)

agentd is the daemon that gives OpenClaw structured, audited, programmatic
access to the full system. OpenClaw talks to agentd, agentd talks to Linux.

**Why a separate daemon instead of just bash?**
- Hash-chained audit log of every system mutation
- Structured API instead of string-parsing shell output
- Capability token minting/verification
- Sandbox orchestration (bubblewrap + systemd transient units)
- Event bus for real-time system state

**API (Unix socket: /run/agentos/agentd.sock):**

```
POST /system/query          # Read any system state (proc, sysctl, services, etc.)
POST /system/mutate         # Any system change (requires approval for destructive ops)
POST /nix/rebuild           # Trigger nixos-rebuild with validation
POST /nix/search            # Search nixpkgs
POST /nix/gc                # Garbage collection
POST /sandbox/exec          # Execute tool in bubblewrap sandbox
POST /sandbox/spawn         # Spawn long-running sandboxed process
POST /capability/mint       # Create short-lived capability token
POST /capability/verify     # Verify token for tool execution
GET  /events                # Stream events (SSE)
GET  /events/log            # Query hash-chained event log
GET  /health                # System health snapshot
```

**Event Log (SQLite, append-only, hash-chained):**

```sql
CREATE TABLE events (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  ts        TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  type      TEXT    NOT NULL,  -- 'nix.rebuild', 'sandbox.exec', 'system.mutate', ...
  actor     TEXT    NOT NULL,  -- 'openclaw.main', 'app.code-simplifier', ...
  payload   TEXT    NOT NULL,  -- JSON: what happened
  prev_hash TEXT    NOT NULL,  -- SHA-256 of previous event
  hash      TEXT    NOT NULL,  -- SHA-256(id || ts || type || actor || payload || prev_hash)
  approval  TEXT              -- null = not needed, 'pending', 'granted', 'denied'
);

CREATE TABLE artifacts (
  id              TEXT PRIMARY KEY,  -- content-addressed: SHA-256 of content
  created_at      TEXT NOT NULL,
  event_id        INTEGER REFERENCES events(id),
  content_type    TEXT NOT NULL,
  size_bytes      INTEGER NOT NULL,
  storage_path    TEXT NOT NULL      -- /var/lib/agentos/artifacts/<id>
);

CREATE TABLE capabilities (
  token       TEXT PRIMARY KEY,
  granted_to  TEXT NOT NULL,  -- app/tool identifier
  permissions TEXT NOT NULL,  -- JSON array: ["net:https://api.github.com", "fs:rw:/tmp/work"]
  minted_at   TEXT NOT NULL,
  expires_at  TEXT NOT NULL,
  revoked     BOOLEAN DEFAULT FALSE
);
```

### 2.2 agentos-bridge — OpenClaw Plugin

A TypeScript OpenClaw plugin that registers agentd's capabilities as
OpenClaw tools. This is how OpenClaw "inherits all APIs."

```typescript
// packages/agentos-bridge/index.ts
// OpenClaw plugin that exposes agentd as tools

export default function agentOSBridge(gateway) {
  // Register tools that talk to agentd over unix socket

  gateway.registerTool('system_query', {
    description: 'Query any system state: processes, services, network, disk, kernel params, logs',
    schema: {
      query: { type: 'string', description: 'What to query: processes|services|network|disk|logs|sysctl|...' },
      args: { type: 'object', description: 'Query-specific arguments' }
    },
    async execute({ query, args }) {
      return await agentdClient.post('/system/query', { query, args });
    }
  });

  gateway.registerTool('system_mutate', {
    description: 'Modify system state: services, firewall, users, mounts, kernel params',
    schema: {
      mutation: { type: 'string' },
      args: { type: 'object' },
      reason: { type: 'string', description: 'Why this change is needed' }
    },
    async execute({ mutation, args, reason }) {
      return await agentdClient.post('/system/mutate', { mutation, args, reason });
    }
  });

  gateway.registerTool('nix_rebuild', {
    description: 'Rebuild NixOS configuration. Validates before applying.',
    schema: {
      changes: { type: 'string', description: 'Description of what changed' },
      dry_run: { type: 'boolean', default: false }
    },
    async execute({ changes, dry_run }) {
      return await agentdClient.post('/nix/rebuild', { changes, dry_run });
    }
  });

  gateway.registerTool('sandbox_exec', {
    description: 'Execute an untrusted tool/script in a bubblewrap sandbox',
    schema: {
      command: { type: 'string' },
      capabilities: { type: 'array', items: { type: 'string' } },
      timeout_sec: { type: 'number', default: 300 }
    },
    async execute({ command, capabilities, timeout_sec }) {
      return await agentdClient.post('/sandbox/exec', { command, capabilities, timeout_sec });
    }
  });

  gateway.registerTool('event_log', {
    description: 'Query the system audit log. See what happened and when.',
    schema: {
      filter: { type: 'object', description: 'Filter by type, actor, time range' },
      limit: { type: 'number', default: 50 }
    },
    async execute({ filter, limit }) {
      return await agentdClient.get('/events/log', { params: { ...filter, limit } });
    }
  });
}
```

**What this means in practice:**

OpenClaw can now do ANYTHING via natural language:

```
User: "What processes are using the most CPU?"
→ OpenClaw calls system_query({ query: 'processes', args: { sort: 'cpu', limit: 10 }})
→ agentd reads /proc, returns structured JSON
→ OpenClaw presents it naturally

User: "Kill that Chrome process eating 4GB"
→ OpenClaw calls system_mutate({ mutation: 'process.kill', args: { pid: 12345 }, reason: 'user requested kill of Chrome using 4GB' })
→ agentd logs event, kills process, returns result

User: "Install Docker and set up a development environment"
→ OpenClaw edits configuration.nix (virtualisation.docker.enable = true; + packages)
→ Calls nix_rebuild({ changes: 'Enable Docker + dev tools', dry_run: true }) first
→ Shows user what will change
→ On approval, calls nix_rebuild({ changes: '...', dry_run: false })
→ Done. Logged. Rollbackable.
```

### 2.3 Capability Runtime (Bubblewrap + systemd)

When OpenClaw needs to run untrusted code (third-party skills, user scripts,
downloaded binaries), it uses the sandbox:

**Bubblewrap provides:**
- Filesystem namespace isolation (tool only sees what's explicitly bound)
- User namespace (runs as nobody inside)
- PID namespace (can't see host processes)
- Network namespace (no network by default)
- PR_SET_NO_NEW_PRIVS (can't escalate)

**systemd transient units provide:**
- CPU/memory/IO limits (cgroups)
- Timeout enforcement (RuntimeMaxSec)
- Working directory isolation (WorkingDirectory + PrivateTmp)
- State directory management (StateDirectory)

**Egress Proxy:**
A tiny localhost-only HTTP CONNECT proxy that:
- Default: no domains allowed
- Capability token specifies allowed domains
- All requests logged to the ledger
- DNS filtered through allowlist

```
┌─────────────────┐     ┌──────────────────┐     ┌──────────────┐
│  Sandboxed Tool  │────▶│  Egress Proxy    │────▶│  Internet    │
│  (bubblewrap)    │     │  localhost:19999  │     │  (filtered)  │
│  no direct net   │     │  domain allowlist │     │              │
└─────────────────┘     └──────────────────┘     └──────────────┘
```

### 2.4 The Exploration Principle

**"All to be explored" means:**

Every API running on the system is discoverable by OpenClaw:

```
User: "What services are running?"
→ agentd enumerates systemd units, returns full list with status, ports, PIDs

User: "I installed Postgres, what can it do?"
→ OpenClaw reads the NixOS Postgres module options, the running config,
  the available databases, the connected clients, the logs

User: "What network ports are open?"
→ agentd reads ss -tlnp, nft list ruleset, returns structured data

User: "Can you integrate with the Prometheus running on port 9090?"
→ OpenClaw hits localhost:9090/api/v1/*, discovers metrics,
  can now query and visualize anything Prometheus exposes

User: "Monitor my Docker containers"
→ OpenClaw talks to Docker socket, lists containers, reads logs,
  monitors resource usage, can start/stop/restart
```

OpenClaw doesn't need pre-built integrations for every service.
It can discover and interact with ANY API on the system because
it has full access. Skills just make common operations smoother.

---

## 3. NIXOS MODULE: THE REAL ONE

```nix
# modules/agentos.nix
{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.agentos;
in {
  options.services.agentos = {
    enable = mkEnableOption "AgentOS - AI-native operating system";

    # --- Gateway (OpenClaw) ---
    openclaw = {
      enable = mkOption { type = types.bool; default = true; };
      package = mkOption { type = types.package; default = pkgs.openclaw; };
      port = mkOption { type = types.port; default = 18789; };
      model = mkOption { type = types.str; default = "anthropic/claude-opus-4-6"; };
      configFile = mkOption { type = types.nullOr types.path; default = null; };
    };

    # --- Agent Kernel Daemon ---
    agentd = {
      package = mkOption { type = types.package; default = pkgs.agentos-agentd; };
      socketPath = mkOption { type = types.str; default = "/run/agentos/agentd.sock"; };
    };

    # --- Capability Runtime ---
    sandbox = {
      enable = mkOption { type = types.bool; default = true; };
      egressProxy = {
        enable = mkOption { type = types.bool; default = true; };
        port = mkOption { type = types.port; default = 19999; };
        defaultAllow = mkOption {
          type = types.listOf types.str;
          default = [
            "cache.nixos.org"
            "channels.nixos.org"
            "github.com"
            "api.anthropic.com"
            "api.openai.com"
          ];
          description = "Domains always allowed through egress proxy";
        };
      };
    };

    # --- State ---
    stateDir = mkOption { type = types.path; default = "/var/lib/agentos"; };
    secretsDir = mkOption { type = types.path; default = "/var/lib/agentos/secrets"; };

    # --- Policy ---
    approvalRequired = mkOption {
      type = types.listOf types.str;
      default = [
        "nix.rebuild"
        "system.user.create"
        "system.user.delete"
        "system.firewall.modify"
        "system.disk.format"
        "system.reboot"
        "system.shutdown"
      ];
      description = "Operations requiring explicit user approval";
    };
  };

  config = mkIf cfg.enable {

    # ===== SYSTEM PACKAGES =====
    environment.systemPackages = with pkgs; [
      cfg.openclaw.package
      cfg.agentd.package
      # agentctl CLI
      pkgs.agentos-agentctl
      # Core system tools OpenClaw needs
      nodejs_22
      git
      jq
      sqlite
      ripgrep
      fd
      htop
      btop
      iotop
      lsof
      strace
      tcpdump
      nmap
      curl
      wget
      file
      tree
      rsync
      zip
      unzip
      # Sandbox tools
      bubblewrap
    ] ++ optionals cfg.sandbox.enable [
      podman
    ];

    # ===== AGENTD SERVICE =====
    systemd.services.agentos-agentd = {
      description = "AgentOS Kernel Daemon";
      wantedBy = [ "multi-user.target" ];
      after = [ "network-online.target" ];
      wants = [ "network-online.target" ];

      serviceConfig = {
        Type = "notify";
        ExecStart = "${cfg.agentd.package}/bin/agentd --socket ${cfg.agentd.socketPath} --state-dir ${cfg.stateDir}";
        Restart = "always";
        RestartSec = 3;
        WatchdogSec = 30;

        # agentd runs as root — it IS the system's brain
        # It mediates ALL system access for OpenClaw
        RuntimeDirectory = "agentos";
        StateDirectory = "agentos";

        # But we still harden what we can
        ProtectClock = true;
        ProtectHostname = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
      };
    };

    # ===== OPENCLAW GATEWAY =====
    systemd.services.agentos-gateway = {
      description = "AgentOS Gateway (OpenClaw)";
      wantedBy = [ "multi-user.target" ];
      after = [ "agentos-agentd.service" ];
      requires = [ "agentos-agentd.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = concatStringsSep " " [
          "${cfg.openclaw.package}/bin/openclaw"
          "gateway"
          "--port ${toString cfg.openclaw.port}"
          "--verbose"
        ];
        Restart = "always";
        RestartSec = 5;

        # Gateway also runs with elevated access
        # It needs to invoke agentd + manage workspace
        StateDirectory = "agentos";
      };

      environment = {
        OPENCLAW_NIX_MODE = "1";
        NODE_ENV = "production";
        AGENTOS_SOCKET = cfg.agentd.socketPath;
        HOME = cfg.stateDir;
      };
    };

    # ===== EGRESS PROXY =====
    systemd.services.agentos-egress = mkIf cfg.sandbox.egressProxy.enable {
      description = "AgentOS Egress Proxy (domain-filtered)";
      wantedBy = [ "multi-user.target" ];
      after = [ "agentos-agentd.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = "${cfg.agentd.package}/bin/agentos-egress --port ${toString cfg.sandbox.egressProxy.port} --state-dir ${cfg.stateDir}";
        Restart = "always";
        RestartSec = 3;

        # Egress proxy is the ONLY path to internet for sandboxed tools
        DynamicUser = true;
        CapabilityBoundingSet = "CAP_NET_BIND_SERVICE";
      };
    };

    # ===== PODMAN FOR SANDBOX =====
    virtualisation.podman = mkIf cfg.sandbox.enable {
      enable = true;
      dockerCompat = true;
      defaultNetwork.settings.dns_enabled = false;  # sandboxed = no DNS
    };

    # ===== FIREWALL =====
    networking.firewall = {
      enable = true;
      allowedTCPPorts = [ ];  # Nothing exposed by default
      # Only loopback for Gateway + egress proxy
    };

    # ===== WORKSPACE SETUP =====
    system.activationScripts.agentos-workspace = ''
      mkdir -p ${cfg.stateDir}/{workspace,ledger,artifacts,caps,secrets}
      mkdir -p ${cfg.stateDir}/workspace/skills
      # Symlink system skills from Nix store
      for skill in ${pkgs.agentos-system-skills}/skills/*; do
        ln -sf "$skill" ${cfg.stateDir}/workspace/skills/$(basename "$skill")
      done
      # Install default templates if not present
      for tmpl in AGENTS.md SOUL.md TOOLS.md; do
        if [ ! -f ${cfg.stateDir}/workspace/$tmpl ]; then
          cp ${./templates}/$tmpl ${cfg.stateDir}/workspace/$tmpl
        fi
      done
    '';
  };
}
```

---

## 4. WHAT OPENCLAW INHERITS (THE FULL SYSTEM API)

Because OpenClaw runs with full access via agentd, it automatically
inherits every capability of every service on the system:

### 4.1 Native Linux APIs (via agentd)
| Domain | What OpenClaw Can Do |
|--------|---------------------|
| **Processes** | List, inspect, kill, trace, resource usage per-PID |
| **Filesystem** | Read/write/search any file, watch for changes, manage permissions |
| **Network** | List connections, manage interfaces, configure DNS, edit firewall |
| **Services** | Start/stop/restart/enable/disable any systemd unit |
| **Users** | Create/delete/modify users and groups |
| **Packages** | Search/install/remove via NixOS declarative config |
| **Kernel** | Read/set sysctl params, view dmesg, kernel module management |
| **Hardware** | USB devices, PCI, GPU, sensors, battery, bluetooth |
| **Storage** | Mount/unmount, SMART data, RAID, LVM, encryption |
| **Logs** | journalctl with full filtering, rotation management |
| **Cron/Timers** | systemd timers, scheduling, one-shot execution |
| **Security** | AppArmor profiles, audit logs, capability management |

### 4.2 Running Service APIs (auto-discovered)
| Service | What OpenClaw Inherits |
|---------|----------------------|
| **Postgres** | SQL queries, user management, backup/restore, replication status |
| **Docker/Podman** | Container lifecycle, images, networks, volumes, logs |
| **Nginx/Caddy** | Vhost config, SSL certs, access logs, upstream health |
| **Redis** | Key operations, memory stats, persistence management |
| **Prometheus** | PromQL queries, target status, alert management |
| **Grafana** | Dashboard management, datasource config |
| **Git** | Repo operations, remote management, hook configuration |
| **SSH** | Authorized keys, connection logs, tunnel management |
| **Tailscale** | Network status, ACLs, exit nodes |
| **Any REST API** | Auto-discover endpoints, interact, configure |

**OpenClaw doesn't need a pre-built plugin for each service.**
It reads configs, hits APIs, parses logs. It figures it out.
Skills just provide optimized workflows for common operations.

### 4.3 The NixOS Superpower

NixOS is the perfect OS for an AI to control because:

1. **Everything is declarative** — OpenClaw edits `configuration.nix`, not scattered config files
2. **Atomic upgrades** — Every change is a new generation; instant rollback on failure
3. **Reproducible** — The same config always produces the same system
4. **Discoverable** — `nixos-option` lists all available options with documentation
5. **Safe** — You literally cannot brick a NixOS system; previous generations always boot

This means OpenClaw can:
- **Safely experiment**: try a config → fails → rollback → try another approach
- **Learn the system**: enumerate all NixOS options, understand what's configurable
- **Explain changes**: "I'm enabling `services.postgresql.enable = true` which will..."
- **Guarantee state**: the system always matches what's in configuration.nix

---

## 5. CLAUDE CODE MASTER PROMPT (DEFINITIVE)

```
You are Claude Code acting as a staff systems engineer building AgentOS.

# THE CORE IDEA
AgentOS is a NixOS distribution where OpenClaw IS the operating system interface.
OpenClaw has FULL system access — root, all files, all processes, all APIs.
It controls everything. It inherits every API on the system.
The sandbox exists only for UNTRUSTED third-party tools, not for OpenClaw itself.

# WHAT TO BUILD (in order)

## M0 — Dev VM That Boots (Week 1)

### Repo skeleton:
./flake.nix
./nix/modules/agentos.nix         # NixOS module
./nix/hosts/dev-vm.nix            # VM configuration
./nix/hosts/iso.nix               # Installer ISO config
./crates/agentd/                  # Rust daemon (kernel bridge)
./crates/agentctl/                # Rust CLI
./crates/agentos-egress/          # Rust egress proxy
./packages/agentos-bridge/        # OpenClaw plugin (TypeScript)
./packages/agentos-system-skills/ # System skills (SKILL.md)
./skills/system-packages/SKILL.md
./skills/system-config/SKILL.md
./skills/system-monitor/SKILL.md
./skills/file-manager/SKILL.md
./skills/network-manager/SKILL.md
./skills/service-explorer/SKILL.md
./templates/AGENTS.md
./templates/SOUL.md
./templates/TOOLS.md
./apps/demo-code-simplifier/      # Demo app as openclawPlugin
./docs/ARCHITECTURE.md
./docs/THREAT_MODEL.md
./docs/CAPABILITIES.md

### flake.nix inputs:
- nixpkgs (unstable)
- nix-openclaw (github:openclaw/nix-openclaw)
- nixos-generators (for ISO building)
- disko (declarative disk partitioning)
- agenix (encrypted secrets)
- home-manager
- crane or naersk (Rust building in Nix)
- flake-utils

### NixOS module (nix/modules/agentos.nix):
Provides services.agentos with:
- agentd as systemd service (root, notify type, watchdog)
- OpenClaw Gateway as systemd service (depends on agentd)
- Egress proxy as systemd service (DynamicUser)
- Workspace activation script (symlinks skills, installs templates)
- Full system packages (all diagnostic/admin tools)
- Podman for container sandbox
- Firewall defaults (nothing exposed)

### Dev VM (nix/hosts/dev-vm.nix):
- QEMU guest, 4GB RAM, 4 CPUs
- Sway (Wayland) desktop
- Auto-login
- OpenClaw Gateway on localhost:18789
- agentd on /run/agentos/agentd.sock
- Firefox + kitty + vscode
- Boot → user sees agent in WebChat

### Build commands:
nix build .#nixosConfigurations.agentos-vm.config.system.build.vm
./result/bin/run-agentos-vm-vm -m 4096 -smp 4

## M0: agentd (Rust)

Minimal API over Unix socket (HTTP+JSON via axum/hyper):

POST /system/query
  - query: "processes" → reads /proc, returns JSON
  - query: "services" → systemctl list-units --type=service --output=json
  - query: "network.connections" → ss -tlnpH, parsed
  - query: "network.interfaces" → ip -j addr
  - query: "disk" → df -BM --output=source,size,used,avail,pcent,target
  - query: "logs" → journalctl --output=json -n <limit> -u <unit>
  - query: "sysctl" → sysctl -a, parsed
  - query: "nixos.options" → nixos-option <path>

POST /system/mutate
  - mutation: "service.restart" + args.unit
  - mutation: "service.stop" + args.unit
  - mutation: "process.kill" + args.pid + args.signal
  - mutation: "file.write" + args.path + args.content
  - mutation: "firewall.add" + args.rule
  - mutation: "user.create" + args.name
  Requires approval if mutation type is in approval_required list.

POST /nix/rebuild
  - Runs: nix flake check (validate)
  - Then: nixos-rebuild switch --flake /etc/nixos#agentos
  - Logs full output
  - Returns: success/failure + generation number

POST /nix/search
  - Runs: nix search nixpkgs#<query> --json

POST /sandbox/exec
  - Constructs bubblewrap invocation:
    bwrap \
      --unshare-all \
      --share-net (only if capability grants network) \
      --ro-bind /nix/store /nix/store \
      --bind /tmp/sandbox-<id> /work \
      --proc /proc \
      --dev /dev \
      --die-with-parent \
      --new-session \
      <command>
  - Wraps in systemd-run --scope for resource limits
  - If network allowed: proxy through agentos-egress

GET /events/log
  - Query SQLite event log with filters

GET /health
  - Returns system snapshot: CPU, RAM, disk, load, services, gateway

Every mutation and sandbox execution creates an Event in the hash-chained log.
prev_hash of event N = hash of event N-1.
hash = SHA-256(id || ts || type || actor || payload || prev_hash).

## M0: agentos-bridge (OpenClaw plugin)

TypeScript plugin that registers tools with OpenClaw Gateway:
- system_query → POST /system/query
- system_mutate → POST /system/mutate
- nix_rebuild → POST /nix/rebuild
- nix_search → POST /nix/search
- sandbox_exec → POST /sandbox/exec
- event_log → GET /events/log
- system_health → GET /health

This plugin connects via the Unix socket. It's loaded by OpenClaw on startup.
Configure in openclaw.json: { "extensions": ["agentos-bridge"] }

## M0: System Skills

### system-packages/SKILL.md
Agent can: search, install (edit config.nix → rebuild), remove, rollback, list generations.
Has full nix access. Understands NixOS declarative model.

### system-config/SKILL.md
Agent can: read and edit any NixOS module option.
Knows to validate before applying. Knows to show diff before rebuild.
Can enumerate available options with nixos-option.

### system-monitor/SKILL.md
Agent can: read CPU/RAM/disk/network/GPU/processes/services/logs.
Presents data naturally. Alerts on thresholds.
Can correlate: "your disk is full because Docker images take 40GB"

### file-manager/SKILL.md
Agent can: ls, find, grep, cat, mv, cp, rm, chmod, chown, tar, zip.
Presents directory contents with context.
Can search file contents with ripgrep. Can preview images (describe).

### network-manager/SKILL.md
Agent can: nmcli (WiFi), ip, ss, nftables, resolvectl, tailscale.
Diagnoses connectivity issues. Configures VPN. Manages firewall.

### service-explorer/SKILL.md
Agent discovers and interacts with any running service.
Reads configs, hits APIs, parses logs. Not pre-programmed per-service.
"What services are running?" → discovers everything.
"Talk to the Postgres on port 5432" → connects, explores.

## M0: Templates

### AGENTS.md — System Agent
You are AgentOS. You ARE the operating system.
Full system access via agentd. Every process, file, network connection,
service, config file — you see it all, you control it all.
Your job: be the best OS interface a human has ever used.
Never break the system without approval. Always validate. Always log.
Explain what you're doing. Show your work.
You can discover and interact with ANY service running on this machine.
You inherit every API. Nothing is hidden from you.

### SOUL.md
You are not an assistant running on a computer. You ARE the computer.
When the user talks to you, they're talking to their machine.
Be the OS they always wished they had.
Calm. Competent. Omniscient about the system. Never flustered.
Diagnose before fixing. Explain before changing. Rollback on failure.
The user's data is sacred.

### TOOLS.md
Documents all agentd endpoints, all system skills, all sandbox capabilities.
Reference for the agent to know what it can do.

## M1 — Secure Tool Execution (Week 2-3)

### Bubblewrap runner:
agentd endpoint POST /sandbox/exec fully implemented:
- Filesystem: only /nix/store (ro) + working dir (rw) + /tmp
- Network: none by default; if capability allows, only through egress proxy
- User: namespace isolation, runs as nobody
- PID: separate namespace
- Seccomp: default filter (block dangerous syscalls)

### systemd transient units:
Every sandbox execution wrapped in:
systemd-run --scope \
  --property=MemoryMax=2G \
  --property=CPUQuota=200% \
  --property=RuntimeMaxSec=300 \
  bwrap [...]

### Egress proxy (agentos-egress):
Tiny Rust HTTP CONNECT proxy:
- Listens on localhost:19999
- Reads capability token from X-Capability header
- Looks up allowed domains for that token
- Connects only to allowed destinations
- Logs all connections to agentd event log
- Returns 403 for disallowed domains

### Integration test:
1. Spawn sandbox with no network capability
2. Tool tries to curl google.com → fails (no network)
3. Spawn sandbox with capability ["net:https://api.github.com"]
4. Tool curls api.github.com → succeeds (through egress proxy)
5. Tool curls google.com → fails (not in allowlist)
6. Verify all attempts logged in event log

## M2 — Ledger + Replay (Week 3-4)

### Hash chain verification:
agentctl verify-ledger → walks entire event chain, verifies hashes.
Any tampering detected = loud alert.

### Replay:
agentctl replay --event-id <N>
Shows: what goal led to this, what plan was made, what tools were called,
what the system state was before and after.

### Artifact store:
Content-addressed storage at /var/lib/agentos/artifacts/
Every file created by agents gets hashed and stored.
Linked to events in the ledger.

## M3 — App Model (Week 4-5)

### agentctl install github:org/app
1. Adds flake input to /etc/nixos/flake.nix
2. Reads openclawPlugin output: { name, skills, packages, needs }
3. Creates state dirs per needs.stateDirs
4. Mounts secret paths per needs.requiredEnv
5. Registers skills in workspace
6. Registers capability requirements
7. nixos-rebuild switch

### Demo app: code-simplifier
A mini "app" that shows the pattern:
- openclawPlugin output with name, skills, packages
- SKILL.md with frontmatter + deterministic steps
- AGENTS.md explaining what it does
- A CLI tool that the skill invokes
- Declared capabilities: ["fs:rw:/tmp/code-simplifier", "net:https://api.github.com"]

### Approvals UI:
When an app requests a capability not yet approved:
- agentd queues it as a pending approval
- OpenClaw notifies the user: "code-simplifier wants network access to github.com. Allow?"
- User approves/denies via chat
- Decision logged in ledger

# CODING STANDARDS

## Rust (agentd, agentctl, egress proxy):
- axum for HTTP over Unix socket
- rusqlite for SQLite
- tokio for async runtime
- serde + serde_json for serialization
- sha2 for hashing
- clap for CLI
- anyhow for error handling
- Tests: cargo test in Nix check

## Nix:
- Flakes only, no channels
- mkOption + mkIf + mkEnableOption consistently
- crane or naersk for Rust builds
- nix flake check must pass
- ISO buildable from flake

## TypeScript (agentos-bridge):
- Follow OpenClaw plugin conventions
- TypeBox schemas for validation
- Communicate with agentd via Unix socket HTTP client

## Skills (SKILL.md):
- YAML frontmatter: name, description, tools, activation
- Markdown body: deterministic instructions for the agent
- Reference agentd tools (system_query, system_mutate, etc.)
- No secrets in skill files ever

# HOW TO RUN (paste at end of output)

# Build and run the VM:
nix build .#nixosConfigurations.agentos-vm.config.system.build.vm
./result/bin/run-agentos-vm-vm -m 4096 -smp 4

# Build installer ISO:
nix build .#nixosConfigurations.agentos-iso.config.system.build.isoImage
ls result/iso/

# Run agentd standalone (for development):
cargo run --manifest-path crates/agentd/Cargo.toml -- --socket /tmp/agentd.sock --state-dir /tmp/agentos

# Run tests:
nix flake check
cargo test --workspace

# Query the ledger:
agentctl events --last 20
agentctl verify-ledger

# Install a demo app:
agentctl install ./apps/demo-code-simplifier
```

---

## 6. SKILL: service-explorer/SKILL.md (THE KEY SKILL)

This is what makes "inherits all APIs" real:

```markdown
---
name: service-explorer
description: >
  Discover and interact with ANY service running on the system.
  Not pre-programmed per-service. Discovers dynamically.
tools:
  - system_query
  - system_mutate
  - sandbox_exec
  - bash
  - read
activation: auto
---

# Service Explorer Skill

You can discover and interact with any service on this system.
You are not limited to pre-built integrations.

## Discovery

### List all running services
```
system_query({ query: "services" })
```

### Find what's listening on which ports
```
system_query({ query: "network.connections", args: { listening: true }})
```

### Read a service's configuration
For NixOS services: `system_query({ query: "nixos.options", args: { path: "services.<name>" }})`
For config files: `bash: cat /etc/<service>/config`

### Read a service's logs
```
system_query({ query: "logs", args: { unit: "<service-name>", lines: 100 }})
```

## Interaction

### HTTP/REST APIs
If a service exposes an HTTP API on localhost:<port>:
```bash
curl -s http://localhost:<port>/ | jq .
curl -s http://localhost:<port>/api/v1/health | jq .
```
Explore endpoints. Read API docs if available.

### Databases
```bash
# PostgreSQL
psql -U postgres -c "SELECT datname FROM pg_database;"
psql -U postgres -d <dbname> -c "\\dt"

# MySQL/MariaDB
mysql -e "SHOW DATABASES;"

# Redis
redis-cli PING
redis-cli INFO
```

### Docker/Podman
```bash
podman ps -a --format json
podman logs <container>
podman stats --no-stream --format json
```

### Message Queues
```bash
# RabbitMQ (management API)
curl -s -u guest:guest http://localhost:15672/api/overview | jq .

# Kafka
kafka-topics.sh --list --bootstrap-server localhost:9092
```

## Principle
You don't need a pre-built plugin for every service.
You have full system access. Read configs. Hit APIs. Parse logs.
Figure it out. Discover. Explore. Report back to the user.
```

---

## 7. REALISTIC DELIVERY PLAN

| Milestone | Duration | Deliverable | Ship Criteria |
|-----------|----------|-------------|---------------|
| **M0** | 1 week | VM boots, agentd responds, OpenClaw connected, one skill works | `nix build` → VM → type "what processes are running" → get answer |
| **M1** | 1-2 weeks | Bubblewrap sandbox + egress proxy + integration test | Run untrusted script, prove it has no network, prove egress filtering works |
| **M2** | 1 week | Hash-chained ledger + replay + verification | `agentctl verify-ledger` passes, `agentctl replay` shows full trace |
| **M3** | 1-2 weeks | App model + demo app + approval flow | `agentctl install ./apps/demo` → skill appears → runs in sandbox → user approves capability |
| **M4** | 1 week | ISO builder + onboarding wizard | Boot ISO → install → first boot → agent greets you |
| **M5** | ongoing | Harden, polish, community | Desktop UI, voice, more skills, ClawHub integration |

**The most important thing:** Ship M0 first. Get the VM booting with OpenClaw
connected to agentd. Once you can type "what's running on my system?" and get
a real answer, everything else is incremental.

---

## 8. WHY THIS WINS

**vs. "Just run OpenClaw on Ubuntu":**
OpenClaw on Ubuntu is a guest. It uses bash. It string-parses. It doesn't own the
system config. It can't safely modify the OS. It can't guarantee rollback.

**AgentOS on NixOS:**
OpenClaw IS the OS. It has a structured API to everything (agentd). It owns the
declarative config. Every change is atomic and rollbackable. Every action is logged
and hash-chained. Third-party tools are sandboxed. The entire system is reproducible.

**vs. "Autonomous AI agents" (AutoGPT style):**
AutoGPT runs tools in a vacuum. No system integration. No audit trail.
No sandboxing. No policy. No rollback.

**AgentOS:**
Every tool call has a capability token, runs in a sandbox, goes through a policy
gate, gets logged to an append-only ledger, and can be replayed for debugging.
The agent has FULL system access but every action is auditable and reversible.

**The unique position: the only OS where the AI doesn't just help you use the OS.
The AI IS the OS.**
