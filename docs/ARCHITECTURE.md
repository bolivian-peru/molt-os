# AgentOS Architecture

## Overview

AgentOS (molt-os) is a NixOS distribution where the AI agent IS the operating system interface. OpenClaw has full system access via agentd, a Rust daemon that provides structured, audited access to every aspect of the Linux system.

## Trust Rings

```
RING 0: OpenClaw + agentd
  Full system access. Root-equivalent. Sees and controls everything.
  Components: OpenClaw Gateway, Pi Agent Runtime, agentd, System Skills, Ledger

RING 1: Approved Apps
  Sandboxed with declared capabilities. No root, no arbitrary filesystem.
  Execution: bubblewrap + systemd transient units
  Network: egress proxy with domain allowlist

RING 2: Untrusted Execution
  Maximum isolation. Working directory + /tmp only. No network.
  User scripts, pip packages, npm installs, random binaries.
```

## Component Architecture

```
┌─────────────────────────────────────────────────────────────┐
│ User (Terminal / Browser / Chat)                             │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│ OpenClaw Gateway (:18789)                                    │
│   Pi Agent Runtime → builds prompt → calls Claude API        │
│   agentos-bridge plugin → registers tools + memory backend   │
│   Memory Backend → ZVEC search → injects into prompt         │
└──────────────────────┬──────────────────────────────────────┘
                       │ HTTP over Unix socket
                       ▼
┌─────────────────────────────────────────────────────────────┐
│ agentd (Rust daemon)                                         │
│   /health          — system metrics (sysinfo)                │
│   /system/query    — structured system queries               │
│   /events/log      — hash-chained audit trail (SQLite)       │
│   /memory/*        — ingest, recall, store, health           │
│                                                              │
│   Ledger: append-only, hash-chained, tamper-evident          │
│   Socket: /run/agentos/agentd.sock                           │
│   State:  /var/lib/agentos/                                  │
└──────────────────────┬──────────────────────────────────────┘
                       │
                       ▼
┌─────────────────────────────────────────────────────────────┐
│ Linux Kernel + NixOS                                         │
│   /proc, /sys, systemd, nftables, journald                  │
│   configuration.nix → nixos-rebuild → atomic generations     │
└─────────────────────────────────────────────────────────────┘
```

## Memory Architecture (M0)

```
User message → OpenClaw → Memory Backend search()
                              │
                              ├─ Embed query (local nomic model, 768-dim)
                              ├─ ZVEC semantic search (single collection)
                              ├─ SQLite FTS5 BM25 keyword search
                              ├─ RRF hybrid merge
                              ├─ Token budget cap (~1500 tokens)
                              └─ Return chunks → inject as <system_memory>

Claude sees memories as context text. Never touches the vector DB.
```

Ground truth: Markdown files at `/var/lib/agentos/memory/`.
ZVEC indexes are derived and always rebuildable.

## Data Flow

1. **User sends message** → OpenClaw Gateway
2. **Memory recall** runs locally (ZVEC + FTS5), ~85ms
3. **Prompt assembled** with memories injected as `<system_memory>`
4. **Claude API call** via OAuth/API key
5. **Claude responds** with text + tool calls
6. **Tool execution** → agentd over Unix socket → structured JSON
7. **Results sent back** to Claude for synthesis
8. **Memory write** — diagnosis/event stored in ledger + ZVEC
9. **Response delivered** to user

## Event Ledger

Every system mutation creates a hash-chained event:

```
Event N:
  hash = SHA-256(id || ts || type || actor || payload || prev_hash)
  prev_hash = hash of Event N-1
```

Genesis event has prev_hash = all zeros. Chain is verifiable with `agentctl verify-ledger`.

## NixOS Integration

AgentOS is a NixOS module (`services.agentos`). One `enable = true` activates:
- agentd systemd service (root, notify type, watchdog)
- OpenClaw Gateway systemd service (depends on agentd)
- Egress proxy (DynamicUser, domain-filtered)
- Workspace activation (skills, templates)
- Firewall defaults (nothing exposed)

NixOS provides atomic, rollbackable system changes — the safest OS for an AI to manage.
