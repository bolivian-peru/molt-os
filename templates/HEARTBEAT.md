# Heartbeat -- Periodic Task Schedule

Defines recurring tasks the agent performs automatically.
Each task has a cadence, a description, and the tools it uses.

## Health Check
- **Cadence**: Every 5 minutes
- **Tool**: `system_health`
- **Action**: Check CPU, memory, disk, load. If any metric exceeds threshold, log a warning event and alert the user on next interaction.
- **Thresholds**:
  - CPU usage > 90% sustained for 3 checks
  - Memory usage > 85%
  - Disk usage > 90% on any mount
  - Load average (1m) > 2x CPU count

## Service Monitor
- **Cadence**: Every 10 minutes
- **Tool**: `service_status`
- **Action**: Check that critical services are running. Log failures to event log.
- **Critical services**:
  - agentd
  - openclaw-gateway
  - sshd
  - {{ADDITIONAL_SERVICES}}

## Log Scan
- **Cadence**: Every 15 minutes
- **Tool**: `journal_logs` (priority: err, since: last scan)
- **Action**: Scan for new errors since last check. Correlate with known issues in memory. Store new patterns.

## Memory Maintenance
- **Cadence**: Every hour
- **Tool**: `memory_recall`, `memory_store`
- **Action**: Review recent events. Consolidate related entries. Update user preference model if new patterns detected.

## NixOS Generation Check
- **Cadence**: Every 30 minutes
- **Tool**: `shell_exec` (nixos-rebuild dry-build)
- **Action**: Verify current generation is healthy. Check for pending updates. Report drift from flake lock.

## Network Watch
- **Cadence**: Every 10 minutes
- **Tool**: `network_info`
- **Action**: Check for unexpected listening ports. Verify expected services are bound. Alert on changes.

## Configuration
- **Enabled**: {{HEARTBEAT_ENABLED}}
- **Log level**: {{HEARTBEAT_LOG_LEVEL}}
- **Alert method**: Event log + next user interaction
