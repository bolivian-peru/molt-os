---
name: morning-briefing
description: >
  Generate a concise daily infrastructure briefing.
  Covers: service health, resource usage, security events, overnight incidents,
  and cost tracking. Designed for Telegram/chat delivery.
tools:
  - system_health
  - service_status
  - journal_logs
  - shell_exec
  - memory_recall
  - event_log
  - teach_patterns
  - teach_context
activation: manual
---

# Morning Briefing Skill

Generate a daily server briefing. Run this as part of the morning heartbeat.

## Briefing Template

Gather data, then compose a message in this format:

```
Good morning. Here's your infrastructure report:

[SERVICE STATUS]
List each critical service with a status emoji:
  green = running and healthy
  yellow = running but degraded
  red = down or failing

[RESOURCE USAGE]
CPU average overnight, current memory, disk usage per mount.
Flag anything above threshold.

[OVERNIGHT EVENTS]
Summarize what happened while the user was asleep.
Include: auto-remediations, config changes, unusual activity.
Reference audit ledger entries.

[SECURITY]
Failed SSH attempts (count + notable IPs).
Any firewall blocks.
Port scan detection.
New or unexpected processes.

[COST]
Estimated daily server cost.
API call costs if tracked.

[SUMMARY]
One-line assessment: "All quiet" or "One incident overnight, resolved."
```

## Data Collection

Run these in sequence:

1. **System health overview**
   ```
   system_health()
   ```

2. **Critical services check**
   ```
   service_status({ service: "agentd" })
   service_status({ service: "openclaw-gateway" })
   service_status({ service: "sshd" })
   service_status({ service: "nginx" })  -- if configured
   ```

3. **Overnight errors** (last 12 hours)
   ```
   journal_logs({ priority: "err", since: "12 hours ago", lines: 50 })
   ```

4. **Security events**
   ```
   journal_logs({ unit: "sshd", since: "12 hours ago", lines: 30 })
   shell_exec({ command: "journalctl -u sshd --since '12 hours ago' | grep 'Failed password\\|Invalid user' | wc -l" })
   ```

5. **Audit log overnight activity**
   ```
   event_log({ limit: 20 })
   ```

6. **Memory — overnight context**
   ```
   memory_recall({ query: "overnight incidents errors fixes", timeframe: "24h" })
   ```

7. **teachd patterns** — check for overnight trends and anomalies detected between conversations
   ```
   teach_patterns({ min_confidence: 0.5 })
   teach_context({ context: "overnight incidents failures resource trends" })
   ```
   Include any detected patterns in the briefing under [OVERNIGHT EVENTS].

8. **NixOS generation**
   ```
   shell_exec({ command: "nixos-rebuild list-generations | tail -5" })
   ```

## Delivery Style

- Be **concise** — this is scanned on a phone
- Use **emojis sparingly** but effectively for status
- **Lead with the worst news** if there is any
- If everything is fine, say so in one line — don't pad
- Include **specific numbers** — "CPU avg 12%" not "CPU was low"
- Reference audit entries by number for traceability

## Example Output

```
Good morning. Infrastructure report for Feb 20:

Services: agentd, openclaw-gateway, sshd, nginx — all running
Resources: CPU avg 11% overnight | RAM 4.1/7.6 GB | Disk 38%

Overnight: Quiet night. No incidents.
  - PostgreSQL connections peaked at 72/100 at 02:41 (normal range)
  - 1 NixOS garbage collection freed 2.3 GB (auto, audit #51)

Security: 7 failed SSH attempts from 3 IPs
  - 185.220.101.33 (Tor exit, 4 attempts) — already blocked
  - 2 other IPs, single attempts each — noise

Cost: ~$0.33/day (Hetzner CX22)

All systems nominal. Have a good day.
```
