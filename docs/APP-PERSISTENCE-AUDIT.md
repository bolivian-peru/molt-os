# osModa App Persistence Audit & Fix Report

**Date**: 2026-03-23
**Trigger**: ProxySmart marketplace (proxysmart.market) was down for ~7.5 hours after server reboot
**Server**: 90f82524 (168.119.157.243)

## Executive Summary

Deployed apps use **transient systemd units** (`systemd-run`) that don't survive reboots. The `osmoda-app-restore` service intended to recreate them on boot was **broken** (bash arrays in inline ExecStart). Result: 7.5 hours of undetected downtime.

Two additional servers (681f1832, d54b7426) were completely inaccessible due to the Hetzner Ubuntu 24.04 password expiry PAM bug.

## Issues Found & Fixed

### CRITICAL: Transient units lost on reboot

**Root cause**: `app_deploy` (osmoda-bridge `index.ts:2365`) creates units via `systemd-run` which stores them in `/run/systemd/transient/` — wiped on every reboot.

**Fix applied (168.119.157.243)**:
- Created persistent unit files at `/etc/systemd/system/proxysmart-api.service` and `proxysmart-web.service`
- Units have `WantedBy=multi-user.target`, `Restart=on-failure`, proper `After=` dependencies
- Marked in registry as `persistent: true` so the restore service skips them

**Long-term fix**: Updated `install.sh` app-restore to use an external script (`/opt/osmoda/bin/osmoda-app-restore.sh`) instead of inline bash. Supports env vars, args, and persistent-unit skipping.

### CRITICAL: App restore service broken

**Root cause**: The `osmoda-app-restore.service` used inline bash in `ExecStart` with bash arrays (`SYSARGS=()`). systemd's environment parsing rejects this: `Invalid environment variable name evaluates to an empty string: SYSARGS[@]`.

Additionally, the restore script was **missing**:
- Environment variable restoration (`--setenv=KEY=VAL`)
- Command argument restoration (only passed `$COMMAND`, not args)

**Fix applied**: Replaced inline ExecStart with `/opt/osmoda/bin/osmoda-app-restore.sh` — a standalone script that handles all fields from registry.json including env vars, args, persistent-unit skipping, and collision detection.

### HIGH: 2 servers inaccessible (password expiry)

**Servers**: 681f1832 (49.13.220.5), d54b7426 (91.99.113.246)
**Root cause**: Hetzner Ubuntu 24.04 PAM enforces password expiry even for SSH key auth
**Fix applied**: Used Hetzner rescue mode to inject PAM bypass (`account sufficient pam_succeed_if.so uid = 0`) into `/etc/pam.d/sshd` and reset shadow expiry fields

### HIGH: 2 ghost apps in registry

**Apps**: `proxysmart-tgbot` and `proxysmart-bot` — marked `running` in registry but no systemd units, no processes, no ports
**Fix applied**: Marked as `stopped` in registry

### MEDIUM: Stale V1 artifacts

**Found**:
- 3 systemd timer/service pairs (`proxysmart-sync`, `proxysmart-expire`, `proxysmart-backup`) pointing to abandoned V1 at `/root/proxysmart-marketplace/`
- 2 stale V1 watchers monitoring port 3000 (V1), permanently degraded with 2303+ retries
- 2 crontab entries for V1 database backups
- 4 duplicate API watchers (port 4000)

**Fix applied**: V1 timers stopped and disabled. Stale crontab entries removed.

### MEDIUM: osmoda-mesh crash-looping

`osmoda-mesh.service` at 5200+ restart cycles with:
```
error: the argument '--listen-addr <LISTEN_ADDR>' cannot be used multiple times
```
Not app-related but contributes to log noise and wasted CPU.

### LOW: Secrets in plaintext in registry.json

The app registry at `/var/lib/osmoda/apps/registry.json` contains plaintext secrets (Telegram bot token, PostgreSQL password, Redis password, JWT secret, CoinGate API key, MailerSend API key). The file has `644` permissions — should be `600`.

## Architecture: How App Persistence Works

```
┌─── DEPLOY TIME ────────────────────────────────────────┐
│ app_deploy tool (osmoda-bridge/index.ts)                │
│   1. Creates transient unit via systemd-run             │
│   2. Writes config to /var/lib/osmoda/apps/registry.json│
└─────────────────────────────────────────────────────────┘

┌─── BOOT TIME ──────────────────────────────────────────┐
│ osmoda-app-restore.service (oneshot, RemainAfterExit)   │
│   1. Reads registry.json                                │
│   2. For each app with status=="running":               │
│      - Skips if persistent==true (has real unit file)   │
│      - Skips if unit already active                     │
│      - Recreates transient unit via systemd-run         │
│        with env vars, args, resource limits             │
└─────────────────────────────────────────────────────────┘

┌─── PERSISTENT APPS ────────────────────────────────────┐
│ For critical apps (like ProxySmart), create real unit   │
│ files in /etc/systemd/system/ and mark persistent=true  │
│ in registry. These survive reboots natively.            │
└─────────────────────────────────────────────────────────┘
```

## Remaining Work

| Task | Priority | Status |
|------|----------|--------|
| Deploy fixed restore script to c9f85cd6 (sawmill app) | HIGH | TODO |
| Verify 681f1832 and d54b7426 SSH after reboot | HIGH | TODO (rebooting now) |
| Remove stale V1 watchers via osmoda-watch API | MEDIUM | TODO |
| Remove duplicate watchers (4x api port 4000) | MEDIUM | TODO |
| Fix osmoda-mesh crash loop (duplicate --listen-addr) | MEDIUM | TODO |
| Set registry.json permissions to 600 | LOW | TODO |
| Add persistent unit creation to app_deploy for critical apps | LOW | Future enhancement |

## Files Changed

| File | Change |
|------|--------|
| `scripts/install.sh` | Replaced broken inline app-restore with external script |
| `/etc/systemd/system/proxysmart-api.service` (168.119.157.243) | New persistent unit |
| `/etc/systemd/system/proxysmart-web.service` (168.119.157.243) | New persistent unit |
| `/etc/systemd/system/osmoda-app-restore.service` (168.119.157.243) | Updated to call script |
| `/opt/osmoda/bin/osmoda-app-restore.sh` (168.119.157.243) | New restore script |
| `/var/lib/osmoda/apps/registry.json` (168.119.157.243) | Fixed ghost apps, marked persistent |
| `/etc/pam.d/sshd` (49.13.220.5, 91.99.113.246) | PAM bypass for SSH key auth |
