---
name: self-healing
description: >
  Detect service failures and system anomalies. Diagnose root causes.
  Auto-remediate using NixOS rollback, service restart, or config repair.
  Every action logged to the hash-chained audit ledger.
tools:
  - system_health
  - service_status
  - shell_exec
  - journal_logs
  - event_log
  - memory_store
  - memory_recall
  - file_read
activation: auto
---

# Self-Healing Skill

You can detect, diagnose, and fix system problems automatically.

## Detection

When you detect a service failure or anomaly during a heartbeat check:

1. **Confirm the failure** — don't act on a single check
   ```
   service_status({ service: "nginx" })
   ```
   If the service is down, check again after 10 seconds. If still down, proceed.

2. **Check journal logs** for why it failed
   ```
   journal_logs({ unit: "nginx", lines: 30, priority: "err" })
   ```

3. **Recall past incidents** — have we seen this before?
   ```
   memory_recall({ query: "nginx failure", timeframe: "30d" })
   ```

## Diagnosis

Determine the root cause before acting:

- **Service crashed** → check logs for segfault, OOM, config error
- **Config file missing/corrupted** → check if NixOS generation has it
- **Dependency failure** → another service it depends on is down
- **Resource exhaustion** → disk full, OOM killer, too many connections
- **Bad deploy** → most recent nixos-rebuild introduced the issue

## Remediation (ordered by safety)

### Level 1: Restart the service
```
shell_exec({ command: "systemctl restart nginx" })
```
Safe, fast, fixes 80% of issues.

### Level 2: NixOS rollback
If restart doesn't fix it, or if config is corrupted:
```
shell_exec({ command: "nixos-rebuild switch --rollback" })
```
This atomically reverts to the last known-good NixOS generation.

### Level 3: Targeted config fix
If you can identify the exact config issue:
```
file_read({ path: "/etc/nixos/configuration.nix" })
```
Fix the config, then rebuild:
```
shell_exec({ command: "nixos-rebuild switch" })
```

### Level 4: Resource relief
If disk/memory is the problem:
```
shell_exec({ command: "nix-collect-garbage -d" })
shell_exec({ command: "journalctl --vacuum-size=500M" })
```

## After remediation

ALWAYS do these three things:

1. **Verify the fix worked**
   ```
   service_status({ service: "nginx" })
   ```

2. **Store in memory**
   ```
   memory_store({
     summary: "nginx failure: config missing, rolled back to gen 47",
     detail: "Full diagnosis and fix details...",
     category: "diagnosis",
     tags: "self-healing,nginx,rollback"
   })
   ```

3. **Log to audit ledger** — this happens automatically through agentd

## Notification

After fixing an issue, compose a clear message for the user:

```
I detected nginx was down at 03:17 UTC.

Cause: The configuration file /etc/nginx/nginx.conf referenced
a missing upstream. This was introduced in NixOS generation 48
(rebuilt 2 hours ago).

Action: Rolled back to generation 47. nginx is running again.
All health checks pass.

Audit entry: #52 (hash: a7f3b2...)
```

Keep it factual, concise, and include the audit reference.

## Critical rules

- NEVER make changes without logging them
- NEVER skip verification after a fix
- If unsure, restart is safer than rollback
- If rollback fails, STOP and alert the user — don't cascade
- Store EVERY incident in memory — patterns build over time
