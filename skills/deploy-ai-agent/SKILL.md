---
name: deploy-ai-agent
description: Deploy and manage AI agent workloads with GPU checks, API key management, and health monitoring
activation: auto
tools:
  - app_deploy
  - app_list
  - app_logs
  - app_stop
  - app_restart
  - app_remove
  - system_discover
  - system_health
  - system_query
  - shell_exec
  - file_write
  - file_read
  - memory_store
  - journal_logs
  - watcher_add
---

# Deploy AI Agent

Deploy AI agent workloads (LangChain, CrewAI, AutoGen, custom frameworks) as managed systemd services with resource monitoring, API key management, and health checks.

## Deploy Workflow

1. **Understand** — Ask what agent framework they're using and what it needs (model provider, API keys, GPU, dependencies)
2. **Check resources** — Use `system_health` to verify RAM, disk, and CPU are sufficient. Check for GPU with `shell_exec` running `nvidia-smi` or `ls /dev/dri`
3. **Set up environment** — Create a Python venv, install Node.js deps, or verify Go binary. Write API keys to the secrets directory.
4. **Deploy** — Use `app_deploy` with appropriate resource limits, environment variables pointing to secrets, and a health-check-friendly port
5. **Verify** — Check `app_logs` for successful startup. Use `system_discover` to confirm the agent's port is listening.
6. **Monitor** — Set up a watcher via `watcher_add` to auto-restart on failure
7. **Remember** — Use `memory_store` to save deployment details for future reference

## Common Patterns

### FastAPI Agent Server (LangChain / LangServe)

```
app_deploy({
  name: "my-agent",
  command: "/var/lib/osmoda/apps/my-agent/venv/bin/uvicorn",
  args: ["app:app", "--host", "0.0.0.0", "--port", "8000"],
  working_dir: "/var/lib/osmoda/apps/my-agent",
  env: {
    ANTHROPIC_API_KEY_FILE: "/var/lib/osmoda/secrets/anthropic-key",
    OPENAI_API_KEY_FILE: "/var/lib/osmoda/secrets/openai-key"
  },
  port: 8000,
  memory_max: "1G",
  cpu_quota: "200%"
})
```

### CrewAI Kickoff

```
app_deploy({
  name: "crew-agent",
  command: "/var/lib/osmoda/apps/crew-agent/venv/bin/python",
  args: ["-m", "crew_agent.main"],
  working_dir: "/var/lib/osmoda/apps/crew-agent",
  env: {
    ANTHROPIC_API_KEY_FILE: "/var/lib/osmoda/secrets/anthropic-key"
  },
  port: 8001,
  memory_max: "2G"
})
```

### Custom Node.js Agent

```
app_deploy({
  name: "node-agent",
  command: "/usr/bin/node",
  args: ["index.js"],
  working_dir: "/home/user/agent",
  env: {
    NODE_ENV: "production",
    PORT: "3000",
    API_KEY_FILE: "/var/lib/osmoda/secrets/agent-api-key"
  },
  port: 3000,
  memory_max: "512M"
})
```

## API Key Management

Never put API keys in environment variables directly. Write them to the secrets directory:

1. Ask the user for their API key
2. `file_write` to `/var/lib/osmoda/secrets/<key-name>` (0600 permissions)
3. Pass the file path as an env var (`*_FILE` convention) or read it in the app's entrypoint
4. The app reads the key from disk at startup

## Resource Checklist

Before deploying, verify:

| Resource | Check | Minimum |
|----------|-------|---------|
| RAM | `system_health` → memory_available | 1 GB free for small agents, 4 GB+ for GPU workloads |
| Disk | `system_health` → disks[0].available | 2 GB for deps + model cache |
| CPU | `system_health` → cpu_usage | 2+ cores recommended |
| GPU | `shell_exec`: `nvidia-smi` or `ls /dev/dri` | Optional — needed for local model inference |
| Python | `shell_exec`: `python3 --version` | 3.10+ for most frameworks |
| Node.js | `shell_exec`: `node --version` | 18+ for modern agent frameworks |

## Health Monitoring

After deployment, set up a watcher:

```
watcher_add({
  name: "my-agent-health",
  check: {
    type: "http_get",
    url: "http://127.0.0.1:8000/health",
    expected_status: 200
  },
  interval_secs: 30,
  actions: ["restart", "notify"]
})
```

For agents without HTTP endpoints, use a process check:

```
watcher_add({
  name: "my-agent-alive",
  check: {
    type: "systemd_unit",
    unit: "osmoda-app-my-agent.service"
  },
  interval_secs: 60,
  actions: ["restart", "notify"]
})
```

## Troubleshooting

- **Agent won't start** — Check `app_logs({ name: "my-agent", lines: 50 })` for Python import errors or missing dependencies
- **Out of memory** — Increase `memory_max` or check if the model is too large for available RAM
- **API key errors** — Verify the key file exists and is readable: `file_read({ path: "/var/lib/osmoda/secrets/anthropic-key" })`
- **Port already in use** — Use `system_discover` to find what's on that port, then pick another
