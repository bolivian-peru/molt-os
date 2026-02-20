---
name: security-hardening
description: >
  Continuous security posture assessment. Scores the system, auto-fixes
  safe issues, proposes fixes for risky ones. Every change audited.
tools:
  - shell_exec
  - system_health
  - journal_logs
  - service_status
  - network_info
  - file_read
  - file_write
  - memory_store
  - memory_recall
  - event_log
activation: auto
---

# Security Hardening Skill

Continuous security scoring and automated hardening.

## Security Scan Checklist

Run these checks and score each one:

### Network (30 points)
```
shell_exec({ command: "ss -tlnp" })
```
- [ ] No unnecessary ports exposed (+10)
- [ ] PostgreSQL/Redis/etc bound to localhost only (+10)
- [ ] Firewall enabled with explicit allowlist (+10)

### SSH (25 points)
```
shell_exec({ command: "sshd -T 2>/dev/null | grep -E 'passwordauth|permitroot|x11forwarding|maxauthtries'" })
```
- [ ] Password auth disabled (+10)
- [ ] Root login disabled or key-only (+5)
- [ ] X11 forwarding disabled (+5)
- [ ] MaxAuthTries <= 3 (+5)

### Failed auth attempts
```
journal_logs({ unit: "sshd", since: "7 days ago", priority: "warning" })
shell_exec({ command: "journalctl -u sshd --since '7 days ago' | grep -c 'Failed password'" })
```

### System (25 points)
- [ ] Automatic security updates enabled (+10)
- [ ] No world-writable files in /etc (+5)
- [ ] NixOS generations < 30 (clean system) (+5)
- [ ] No setuid binaries outside expected set (+5)

### Audit (20 points)
- [ ] agentd ledger intact (hash chain valid) (+10)
- [ ] All recent changes have audit entries (+10)

## Scoring

Calculate: `(earned_points / 100) * 100`

Present as:
```
Security Score: 78/100

Auto-fixed (safe, no approval needed):
  ✅ Enabled fail2ban (NixOS config added)
  ✅ Set MaxAuthTries to 3
  ✅ Blocked 5 IPs with repeated failed logins

Needs your approval:
  ⚠️ Port 5432 exposed publicly — close it? (saves 10 pts)
  ⚠️ Enable automatic nixpkgs security channel

Won't touch without discussion:
  🔴 Root SSH enabled (you may need this for deploys)
```

## Auto-Fix Actions (safe, always do these)

### Enable fail2ban
```nix
services.fail2ban = {
  enable = true;
  maxretry = 3;
  bantime = "1h";
};
```

### Block brute-force IPs
```
shell_exec({ command: "journalctl -u sshd --since '24h ago' | grep 'Failed' | awk '{print $(NF-3)}' | sort | uniq -c | sort -rn | head -10" })
```
For IPs with > 10 attempts:
```nix
networking.firewall.extraCommands = ''
  iptables -A INPUT -s <IP> -j DROP
'';
```

### Close unused ports
```nix
networking.firewall.allowedTCPPorts = [ 22 80 443 ];
# Remove any port that doesn't have a matching service
```

## Approval-Required Actions

Present a clear diff and explain the security impact:
- Switching SSH to certificate auth
- Enabling automatic updates
- Changing firewall rules that affect application access

## Weekly Report

Generate a trending security report:
```
Security Report — Week of Feb 17

Score: 78/100 (↑ from 65 last week)

This week:
- Blocked 142 brute-force SSH attempts from 23 IPs
- Applied 2 NixOS security patches (OpenSSH, curl)
- Closed port 3306 (MySQL was accidentally exposed)
- 0 incidents, 0 unauthorized changes

Trend: Improving. Main gap: root SSH still enabled.
```

Store the report in memory for historical tracking.
