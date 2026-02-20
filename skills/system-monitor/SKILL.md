---
name: system-monitor
description: >
  Monitor system health: CPU, memory, disk, network, processes, services, and logs.
  Present data naturally. Correlate issues across subsystems.
  Alert on thresholds. Diagnose root causes.
tools:
  - system_query
  - system_health
  - memory_recall
  - memory_store
activation: auto
---

# System Monitor Skill

You can monitor every aspect of the system in real time.

## Quick Health Check

```
system_health()
```
Returns CPU usage per core, memory (total/used/available), swap, disk usage per mount, load average, and uptime.

## Process Monitoring

### List processes by resource usage
```
system_query({ query: "processes", args: { sort: "cpu", limit: 20 } })
system_query({ query: "processes", args: { sort: "memory", limit: 20 } })
```

### Investigate a specific process
```
system_query({ query: "processes", args: { name: "firefox" } })
```

## Disk Usage

```
system_query({ query: "disk" })
```
Returns per-mount: filesystem type, total/used/available space, usage percentage.

## Diagnosis Workflow

When the user reports a problem:

1. **Check health first** — `system_health()` for the overview
2. **Check memory** — recall past similar issues: `memory_recall({ query: "similar problem description" })`
3. **Drill into specifics** — processes, disk, network as needed
4. **Correlate** — "your disk is full because Docker images take 40GB"
5. **Store the diagnosis** — `memory_store({ summary: "...", detail: "...", category: "diagnosis" })`

Always diagnose before suggesting fixes. Explain what you found. Store diagnoses for future reference — next time the same issue occurs, you'll recall it instantly.

## Thresholds

Flag these to the user proactively:
- CPU sustained >90% for >5 minutes
- Memory usage >85%
- Swap usage >50%
- Any disk mount >90% full
- Load average > 2x number of cores
