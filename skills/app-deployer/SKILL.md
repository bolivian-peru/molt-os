---
name: app-deployer
description: Deploy and manage user applications as managed systemd services
activation: auto
tools:
  - app_deploy
  - app_list
  - app_logs
  - app_stop
  - app_restart
  - app_remove
  - system_discover
  - journal_logs
  - memory_store
---

# App Deployer

Deploy and manage user applications (Node.js apps, Python scripts, Go binaries, Docker-less containers, etc.) as first-class managed processes on the OS.

## Deploy Workflow

1. **Understand** — Ask what to deploy. Determine the command, working directory, environment variables, and resource requirements.
2. **Deploy** — Use `app_deploy` with appropriate parameters. Default isolation uses `DynamicUser=yes` (ephemeral UID, no root).
3. **Verify** — Use `app_list` to confirm the app is running. Check `app_logs` for startup output.
4. **Store** — Use `memory_store` to remember what was deployed and why.

### Example

```
User: "Deploy my Node.js API at /home/user/api"

1. app_deploy({
     name: "user-api",
     command: "/usr/bin/node",
     args: ["server.js"],
     working_dir: "/home/user/api",
     env: { NODE_ENV: "production", PORT: "3000" },
     port: 3000,
     memory_max: "256M",
     user: "user"
   })
2. app_list() → confirm running, check PID
3. app_logs({ name: "user-api", lines: 20 }) → verify startup
4. memory_store({ summary: "Deployed user-api Node.js app", ... })
```

## Managing Apps

- **Status**: `app_list` shows all apps with live systemd state, PID, memory, CPU usage
- **Logs**: `app_logs` pulls from journald — supports `--since`, `--priority`, line limits
- **Restart**: `app_restart` — if the unit is active, calls `systemctl restart`. If inactive, re-deploys from registry
- **Stop**: `app_stop` — stops the unit, marks as stopped in registry (preserved for restart)
- **Remove**: `app_remove` — stops and permanently deletes from registry

## Resource Limits

Apps run under systemd cgroups. Available limits:

| Parameter | Example | Effect |
|-----------|---------|--------|
| `memory_max` | `"256M"`, `"1G"` | Hard memory cap via MemoryMax= |
| `cpu_quota` | `"50%"`, `"200%"` | CPU time limit (200% = 2 cores) |

## Isolation

- **Default** (`DynamicUser=yes`): App runs as an ephemeral system user. No root, no access to other users' files. Best for self-contained binaries and services.
- **Named user** (`user: "username"`): App runs as the specified user. Use when the app needs to read/write files owned by that user (e.g., `/home/user/data`).

## Boot Persistence

Apps are registered in `/var/lib/osmoda/apps/registry.json`. The `osmoda-app-restore` service re-creates all running apps as transient systemd units on boot. Apps marked as `stopped` are not restored.

## Best Practices

- Always specify `memory_max` for production apps to prevent OOM issues
- Use `port` parameter for discovery — `system_discover` will find the app
- Use named `user` when the app needs filesystem access beyond its own binary
- Check `app_logs` after deploy to catch startup errors immediately
- Use `app_restart` (not stop + deploy) to preserve the same configuration
