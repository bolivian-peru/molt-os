# osModa Production Roadmap — Full System Fix Plan

**Date**: 2026-03-27
**Status**: NOT PRODUCTION READY
**Goal**: Make osModa a properly working NixOS agentic management system

---

## Current Reality vs Architecture Promise

| Architecture says | Reality |
|---|---|
| "NixOS distribution with AI-native system management" | All 5 running servers are Ubuntu 24.04 |
| "Requires a real NixOS system" | install.sh runs on Ubuntu, NixOS path is broken |
| NixOS module (`services.osmoda`) wires everything | Module exists but is never used — install.sh writes raw systemd units |
| "NixOS provides atomic, rollbackable system changes" | No server has NixOS generations, no rollbacks possible |
| 10 Rust daemons (agentd, keyd, watch, routines, mesh, voice, mcpd, teachd, egress) | Daemons are built but crash-loop on some servers (mesh: 5200+ restarts) |
| "SafeSwitch deploys with auto-rollback" | Can't work without NixOS generations |
| "Hash-chained audit ledger, 321+ events" | Ledger works on servers where agentd runs |
| x402 payment-gated API | x402 deps not installed, graceful fallback (manual payment) |
| Agent Card (EIP-8004) | Agent card serves but WS endpoint broken in nginx |

---

## Phase 0: EMERGENCY (Do Today) — Stop the Bleeding

### 0.1 Revert NixOS snapshot → Ubuntu for new servers
**Why**: Every server spawned since March 24 is dead on arrival.
**Action**: Change `image: "369677089"` back to `image: "ubuntu-24.04"` in server.js and re-add `--skip-nixos`.
**Time**: 5 minutes.

### 0.2 Fix/delete dead orders
- `3ff18436` — Hetzner server deleted 28 days ago, order still "running". Mark deleted.
- `50655a31` — Spawned on NixOS snapshot, never installed. Delete Hetzner server + mark deleted.

### 0.3 SSL cert auto-renewal
**Action**: Add certbot systemd timer on spawn server.
```bash
systemctl enable certbot.timer
systemctl start certbot.timer
```

### 0.4 Fix nginx WS upgrade for /api/v1/chat
**Why**: The agent card advertises `wss://spawn.os.moda/api/v1/chat/{orderId}` but nginx doesn't proxy WebSocket upgrades for this path. A2A agent-to-agent integration is broken.
**Action**: Add nginx location block with `proxy_set_header Upgrade`, `proxy_set_header Connection "upgrade"`.

---

## Phase 1: PROPER NixOS Snapshot (This Week)

The current NixOS snapshot fails because cloud-init's user_data bash script uses Ubuntu tools that don't exist on NixOS. The fix: **bake osModa into the snapshot itself**.

### 1.1 Build a "golden" NixOS snapshot with osModa pre-installed

Instead of: NixOS base → cloud-init runs install.sh → osModa installs
Do: NixOS + osModa pre-installed → cloud-init only injects config (API key, order ID, heartbeat secret)

**NixOS configuration.nix for the snapshot:**
```nix
{ config, pkgs, ... }:
{
  imports = [ ./hardware-configuration.nix ];

  boot.loader.grub = { enable = true; device = "/dev/sda"; };
  networking.hostName = "osmoda";
  networking.useDHCP = true;
  networking.firewall.allowedTCPPorts = [ 22 80 443 18800 ];

  services.openssh = {
    enable = true;
    settings = {
      PermitRootLogin = "prohibit-password";
      PasswordAuthentication = false;
    };
  };

  # Cloud-init for Hetzner user_data (config injection only)
  services.cloud-init = {
    enable = true;
    network.enable = true;
  };

  # No password = no expiry = no PAM lockout
  users.users.root = {
    hashedPassword = "";
    openssh.authorizedKeys.keys = [
      "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEFkETMpYv/ykTTKJ/50HnplbnZN3Ud4w7H9wZWkGe6L spawn@os.moda"
    ];
  };

  # PAM: don't enforce password expiry for SSH
  security.pam.services.sshd.text = lib.mkForce ''
    account sufficient pam_succeed_if.so uid = 0
    account required pam_unix.so
    auth sufficient pam_unix.so likeauth nullok
    auth required pam_deny.so
    session required pam_unix.so
    session required pam_env.so
  '';

  # Pre-installed packages
  environment.systemPackages = with pkgs; [
    git curl wget jq openssl htop tmux vim nano
    cargo rustc gcc pkg-config gnumake openssl.dev
    nodejs_22 python3
    iproute2 iptables nftables
    shadow  # provides passwd, chage, useradd
  ];

  # Nix settings
  nix.gc = { automatic = true; dates = "weekly"; options = "--delete-older-than 14d"; };
  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  nixpkgs.config.allowUnfree = true;

  system.stateVersion = "24.11";
}
```

**Key differences from current broken snapshot:**
- `users.users.root.hashedPassword = ""` — no password, no expiry, no PAM lockout
- `security.pam.services.sshd.text` — explicit PAM config that bypasses account expiry for root
- `shadow` package added — provides `passwd`, `chage` that install.sh needs
- `python3` added — heartbeat app detection needs it
- `openssl` already included — heartbeat HMAC auth needs it

### 1.2 Fix install.sh for NixOS compatibility

**Bugs to fix:**
```
Line 857-858:  ExecStartPre=/bin/mkdir → /run/current-system/sw/bin/mkdir
Line 1049:     #!/bin/bash → #!/usr/bin/env bash
Line 1352:     ExecStart=/usr/bin/env node → ExecStart=/run/current-system/sw/bin/node
Line 1545:     systemctl is-active agentd → systemctl is-active osmoda-agentd
Line 2211:     Remove dangling 2>&1 || warn
```

**NixOS-aware path detection:**
```bash
# At top of install.sh, detect NixOS and set correct paths
if [ -f /etc/NIXOS ]; then
  MKDIR="/run/current-system/sw/bin/mkdir"
  NODE_BIN="/run/current-system/sw/bin/node"
  BASH_BIN="/run/current-system/sw/bin/bash"
else
  MKDIR="/bin/mkdir"
  NODE_BIN="/usr/bin/node"
  BASH_BIN="/bin/bash"
fi
```

### 1.3 Simplified cloud-init for NixOS (config injection only)

Since osModa is pre-installed in the snapshot, cloud-init only needs to:
1. Write config files (order ID, heartbeat secret, API key)
2. Start services

```bash
#!/bin/bash
# osModa cloud-init — NixOS snapshot config injection
exec > >(tee -a /var/log/osmoda-cloud-init.log) 2>&1

# SSH key injection (NixOS-compatible: append to authorized_keys)
mkdir -p /root/.ssh && chmod 700 /root/.ssh
cat <<'SSHEOF' >> /root/.ssh/authorized_keys
${allKeys}
SSHEOF
chmod 600 /root/.ssh/authorized_keys

# Write osModa config
mkdir -p /var/lib/osmoda/config
echo '${orderId}' > /var/lib/osmoda/config/order-id
echo '${heartbeatSecret}' > /var/lib/osmoda/config/heartbeat-secret
echo 'https://spawn.os.moda/api/heartbeat' > /var/lib/osmoda/config/callback-url
chmod 600 /var/lib/osmoda/config/*

# Run install.sh (handles remaining setup: OpenClaw, ws-relay, etc.)
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | bash -s -- --order-id '${orderId}' --callback-url 'https://spawn.os.moda/api/heartbeat' --heartbeat-secret '${heartbeatSecret}'${apiKeyArgs}
```

### 1.4 Build + test + take new snapshot

1. Create temp CX23 in fsn1
2. Boot rescue, install NixOS with the config above
3. Boot into NixOS, verify SSH works, cloud-init works
4. Run install.sh manually, verify daemons start
5. Take snapshot
6. Test: spawn a server with the snapshot via API, verify full install completes
7. Switch server.js to new snapshot ID

---

## Phase 2: install.sh Quality (This Week)

### 2.1 Fix all NixOS-breaking bugs (listed in 1.2)

### 2.2 Add NixOS path detection at top of script

### 2.3 Fix app-restore shebang
`#!/bin/bash` → `#!/usr/bin/env bash`

### 2.4 Fix heartbeat agentd service name
`systemctl is-active agentd` → `systemctl is-active osmoda-agentd`

### 2.5 Add integration test
Script at `scripts/test-install.sh` that:
- Creates a temp Hetzner CX23 from NixOS snapshot
- Runs install.sh
- Verifies all daemons start
- Verifies heartbeat sends
- Verifies SSH works
- Cleans up

---

## Phase 3: Spawn Server Hardening (Next Week)

### 3.1 Stale order cleanup
- Cron job to detect orders where Hetzner server is deleted but order is "running"
- Auto-mark as deleted, notify admin

### 3.2 Billing status sync
- When order is deleted, set `billing_status: "cancelled"`
- Sweep existing 36 deleted orders with billing_status=active

### 3.3 Nginx v1 WebSocket fix
Add location block for `/api/v1/chat/` with WebSocket upgrade headers.

### 3.4 Kill orphan processes
- PM2 God Daemon (running since Feb 28)
- Stale node process reading orders.enc

### 3.5 Monitoring
- Add uptimerobot or similar for spawn.os.moda
- Alert on: cert expiry < 30 days, disk > 80%, spawn-app crash, heartbeat gap > 5 min

---

## Phase 4: OAuth + API Key Handling (Next Week)

### 4.1 Document OAuth vs API key behavior
- OAuth tokens (`sk-ant-oat01-`) have low rate limits through OpenClaw's proxy
- API keys (`sk-ant-api01-`) have standard rate limits via direct API
- Dashboard should show which type is configured and warn about OAuth limits

### 4.2 Reduce background API consumption
- Default CRM cron: 30 min (not 5 min)
- No duplicate cron jobs
- Heartbeat checks should NOT use the AI agent (use direct SQL/shell)

### 4.3 Rate limit circuit breaker
- If 3 consecutive rate limit errors, pause agent for 5 minutes
- Prevents the retry death spiral that exhausted budgets for days

---

## Phase 5: True NixOS Module Integration (Month 2)

### 5.1 Use the actual NixOS module (osmoda.nix)
Currently install.sh writes raw systemd units. The NixOS module at `nix/modules/osmoda.nix` already defines everything properly. On NixOS servers, install.sh should:
1. Copy the module to `/etc/nixos/osmoda.nix`
2. Add `imports = [ ./osmoda.nix ];` to configuration.nix
3. Set `services.osmoda.enable = true;`
4. Run `nixos-rebuild switch`

This gives:
- Declarative service management
- NixOS generations (rollback!)
- SafeSwitch actually works
- Proper systemd hardening from the module

### 5.2 Migrate existing Ubuntu servers to NixOS
For existing customers who want NixOS:
1. Take a backup
2. Run nixos-infect (with the $bootFs fix)
3. The Phase 2 systemd service handles post-reboot installation
4. This is opt-in, not forced

---

## Phase 6: Full Production Checklist (Month 2)

- [ ] All new servers boot NixOS from snapshot
- [ ] install.sh works on both Ubuntu and NixOS without errors
- [ ] All daemons start and stay running (no crash loops)
- [ ] Heartbeat works on NixOS (openssl, python3 in PATH)
- [ ] WS relay works on NixOS (node in systemd PATH)
- [ ] App-restore works on NixOS (correct shebang + paths)
- [ ] SSL auto-renewal configured
- [ ] Nginx WebSocket upgrade for all paths
- [ ] No ghost/stale orders
- [ ] OAuth rate limit documented and handled gracefully
- [ ] Agent card endpoints all functional
- [ ] Integration test passes on fresh NixOS server
- [ ] GitHub repo install.sh matches deployed version

---

## Priority Matrix

| Task | Impact | Effort | Do When |
|---|---|---|---|
| Revert to Ubuntu (stop broken spawns) | CRITICAL | 5 min | NOW |
| Fix dead orders | HIGH | 10 min | NOW |
| SSL auto-renewal | HIGH | 5 min | NOW |
| Build proper NixOS snapshot | CRITICAL | 2-3 hours | Today |
| Fix install.sh NixOS bugs | HIGH | 1 hour | Today |
| Nginx WS fix | MEDIUM | 15 min | Today |
| Rate limit circuit breaker | MEDIUM | 1 hour | This week |
| NixOS module integration | HIGH | 1 day | Next week |
| Integration test script | MEDIUM | 2 hours | Next week |
| Monitoring/alerting | MEDIUM | 1 hour | Next week |
