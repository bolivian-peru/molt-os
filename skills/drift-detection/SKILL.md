---
name: drift-detection
description: >
  Detect configuration drift — manual changes that exist outside NixOS management.
  Offer to bring imperative changes into declarative config.
tools:
  - shell_exec
  - file_read
  - file_write
  - memory_store
  - memory_recall
activation: auto
---

# Configuration Drift Detection

NixOS is declarative — the config should be the single source of truth.
But reality drifts: manual edits, imperative installs, ad-hoc cron jobs.
Detect and reconcile.

## Checks

### 1. Imperatively installed packages
```
shell_exec({ command: "nix-env -q 2>/dev/null || echo 'none'" })
```
If packages found: offer to add them to `environment.systemPackages` in NixOS config.

### 2. Manual files outside NixOS
```
shell_exec({ command: "find /etc -newer /etc/NIXOS -not -path '/etc/nixos/*' -not -path '/etc/resolv.conf' -type f 2>/dev/null | head -20" })
```
Files modified after last NixOS rebuild may be manual edits.

### 3. Manual cron jobs
```
shell_exec({ command: "ls /etc/cron.d/ /var/spool/cron/crontabs/ 2>/dev/null" })
```
Offer to convert to systemd timers in NixOS config.

### 4. Stale NixOS generations
```
shell_exec({ command: "nixos-rebuild list-generations 2>/dev/null | wc -l" })
```
More than 20 generations → suggest cleanup.

### 5. Orphaned systemd units
```
shell_exec({ command: "systemctl list-units --state=failed --no-pager" })
```

## Remediation

For each drift finding, offer to bring it into NixOS:

```
Drift Report:

⚠️ 3 packages installed via nix-env: htop, ncdu, tree
   → Add to environment.systemPackages? [Y/n]

⚠️ /etc/cron.d/backup exists outside NixOS
   → Convert to systemd timer in NixOS config? [Y/n]

⚠️ 47 old NixOS generations (using 18GB)
   → Keep current + last 5, remove rest? [Y/n]

✅ No unauthorized file changes in /etc/nixos/
✅ All systemd services match NixOS config
```

## Why This Matters

On Ubuntu, configuration drift is invisible and irreversible.
On NixOS with osModa, you can:
1. **Detect** that something was changed manually
2. **Reconcile** by adding it to the declarative config
3. **Prove** with the audit ledger that nothing unauthorized happened
4. **Reproduce** the exact system state on new hardware

This is SOC2/compliance gold.
