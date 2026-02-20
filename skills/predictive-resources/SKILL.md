---
name: predictive-resources
description: >
  Predict resource exhaustion (disk, memory, swap) using trend analysis.
  Proactively alert and propose NixOS config fixes before things break.
tools:
  - system_health
  - shell_exec
  - memory_store
  - memory_recall
  - file_read
  - file_write
activation: auto
---

# Predictive Resource Exhaustion

Don't wait for things to break. Predict when they will and fix proactively.

## Disk Growth Analysis

Collect data points over time and project forward:

```
shell_exec({ command: "df -h --output=target,used,avail,pcent / /var /nix/store /tmp 2>/dev/null || df -h" })
```

Track in memory — store periodic snapshots:
```
memory_store({
  summary: "Disk snapshot: / 38% used, /nix/store 12GB",
  detail: "Full df output...",
  category: "system.config",
  tags: "disk,snapshot,predictive"
})
```

When you have 2+ data points, calculate growth rate:
- Growth per day = (current_used - previous_used) / days_between
- Days until 95% = (capacity * 0.95 - current_used) / growth_per_day

## Thresholds & Alerts

| Metric | Warning | Critical | Action |
|--------|---------|----------|--------|
| Disk fills in < 7 days | Alert user | Auto-clean | `nix-collect-garbage`, logrotate |
| Memory avg > 80% for 1hr | Alert user | OOM risk | Identify top consumers |
| Swap usage growing | Monitor | > 50% used | Recommend more RAM or optimize |
| /nix/store > 30GB | Suggest cleanup | > 50GB | Auto-GC old generations |

## Remediation (NixOS-native)

### Disk: Nix garbage collection
```
shell_exec({ command: "nix-collect-garbage --delete-older-than 14d" })
```

### Disk: Log rotation via NixOS config
Propose adding to configuration.nix:
```nix
services.journald.extraConfig = "SystemMaxUse=500M";
```

### Memory: Identify and advise
```
shell_exec({ command: "ps aux --sort=-%mem | head -15" })
```
Don't kill processes without permission. Advise the user.

### Swap: NixOS swap config
```nix
swapDevices = [{ device = "/swapfile"; size = 4096; }];
```

## User Preference Learning

After each alert, remember what the user chose:
```
memory_store({
  summary: "User prefers automatic log cleanup when disk > 80%",
  detail: "Approved automatic journalctl vacuum and nix-collect-garbage",
  category: "user_pattern",
  tags: "preference,disk,auto-remediation"
})
```

Next time, check memory before asking:
```
memory_recall({ query: "user preference disk cleanup auto" })
```
If they previously approved auto-remediation, do it and just notify.
