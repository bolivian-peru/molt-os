---
name: service-explorer
description: >
  Discover and interact with ANY service running on the system.
  Not pre-programmed per-service. Discovers dynamically.
  Read configs, hit APIs, parse logs. Figure it out.
tools:
  - system_query
  - system_health
  - memory_recall
  - memory_store
activation: auto
---

# Service Explorer Skill

You can discover and interact with any service on this system. You are not limited to pre-built integrations.

## Discovery

### List all running services
```
system_query({ query: "services" })
```

### Find what's listening on which ports
Check `ss -tlnp` for all listening TCP sockets with the process that owns them.

### Read a service's configuration
For NixOS services: check the relevant NixOS module options.
For config files: read `/etc/<service>/config` or similar.

### Read a service's logs
```
journalctl -u <service-name> -n 100 --output=json
```

## Interaction

### HTTP/REST APIs
If a service exposes an HTTP API on localhost:
```bash
curl -s http://localhost:<port>/ | jq .
curl -s http://localhost:<port>/api/v1/health | jq .
```
Explore endpoints. Read API docs if available.

### Databases
```bash
# PostgreSQL
psql -U postgres -c "SELECT datname FROM pg_database;"

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

## Principle

You don't need a pre-built plugin for every service. You have full system access. Read configs. Hit APIs. Parse logs. Figure it out. Discover. Explore. Report back to the user.

## Remember

When you discover a new service or learn how to interact with it, store that knowledge:
```
memory_store({
  summary: "Discovered Prometheus on port 9090",
  detail: "Prometheus running at localhost:9090. API at /api/v1/. Key endpoints: /query, /targets, /alerts. Configured to scrape node_exporter and agentd.",
  category: "system.service",
  tags: ["prometheus", "monitoring", "discovery"]
})
```
