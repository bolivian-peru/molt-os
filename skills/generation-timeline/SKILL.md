---
name: generation-timeline
description: >
  NixOS generation-aware debugging and time-travel. Correlate system state
  changes with application failures. Query "what changed and when."
tools:
  - shell_exec
  - event_log
  - journal_logs
  - memory_recall
  - memory_store
activation: auto
---

# Generation Timeline — Time-Travel Debugging

Correlate NixOS generations + audit ledger + journal logs to debug issues across time.

## "What changed?" Workflow

When a user reports "something broke":

### 1. Get the generation timeline
```
shell_exec({ command: "nixos-rebuild list-generations 2>/dev/null | tail -10" })
```

### 2. Get the audit ledger for the same period
```
event_log({ limit: 30 })
```

### 3. Correlate with journal errors
```
journal_logs({ priority: "err", since: "24 hours ago", lines: 50 })
```

### 4. Check memory for related context
```
memory_recall({ query: "config change rebuild generation", timeframe: "7d" })
```

### 5. Build the timeline

Present a unified view:
```
Timeline — Last 48 Hours:

Feb 19 09:30  Gen 48 applied (nixpkgs update: OpenSSL 3.1→3.2)
Feb 19 09:47  ERROR: app returned first 502 (17 min after gen 48)
Feb 19 09:48  WARNING: 12 more 502s in next minute
Feb 19 09:52  agentd detected service degradation (audit #134)
Feb 19 10:00  You: "something broke"

Root cause: Gen 48 updated OpenSSL, breaking TLS cipher
used by your app.

Options:
(a) Rollback to Gen 47 (reverts everything since)
(b) Pin OpenSSL 3.1 for your app only (overlay)
(c) Update app TLS config (best long-term fix)
```

## "Explain my server" Workflow

When asked "what does this server do" or "explain the setup":

```
shell_exec({ command: "cat /etc/nixos/configuration.nix" })
shell_exec({ command: "nixos-rebuild list-generations | head -5" })
shell_exec({ command: "systemctl list-units --type=service --state=running --no-pager" })
```

Generate a plain-English overview:
```
Your Server — MoltOS (Hetzner CX22)

Services running:
  - agentd (AgentOS system daemon) — since boot
  - OpenClaw Gateway (AI agent interface) — port 18789
  - SSH (key-auth only) — port 22
  - nginx (reverse proxy) — ports 80, 443

Architecture:
  Internet → nginx (443) → OpenClaw (18789)
  SSH → direct access (22)

NixOS: Generation 50, built Feb 20
Resources: 2 CPU, 7.6GB RAM, 80GB disk (38% used)
Uptime: 12 hours

History: 50 generations over 18 days.
Key changes stored in audit ledger (134 entries).
```

## "Fork my server" Workflow

When asked to create a staging copy or export the config:

```
shell_exec({ command: "cat /etc/nixos/configuration.nix" })
shell_exec({ command: "cat /etc/nixos/hardware-configuration.nix" })
```

Generate a self-contained flake that reproduces this machine:
- Include all service configs
- EXCLUDE secrets, API keys, database contents
- Include a README explaining how to deploy

## Critical Rule

The audit ledger is the source of truth for "why" something was done.
NixOS generations are the source of truth for "what" the system state is.
Journal logs are the source of truth for "when" things happened.

Always correlate all three when debugging.
