#!/usr/bin/env bash
# =============================================================================
# osModa Installer — One command to give your computer a brain
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh | bash
#
# Or with options:
#   curl -fsSL ... | bash -s -- --skip-nixos --api-key sk-ant-...
#
# What this does:
#   1. Converts your server to NixOS (via nixos-infect) — auto on Ubuntu/Debian
#      Server reboots into NixOS, then Phase 2 installs daemons automatically.
#   2. Installs Rust toolchain + builds agentd
#   3. Installs OpenClaw AI gateway
#   4. Sets up the osmoda-bridge plugin (90 system tools)
#   5. Installs agent identity + skills
#   6. Starts everything — agentd + OpenClaw
#
# Supports: Ubuntu 22.04+, Debian 12+, existing NixOS
# Tested on: Hetzner Cloud, DigitalOcean, bare metal
#
# NOT supported: Docker, LXC, WSL, OpenVZ, or any container environment.
# osModa is a full NixOS distribution — it needs a real VM or bare metal.
# =============================================================================

set -eo pipefail

# Ensure HOME is set (cloud-init may not set it)
export HOME="${HOME:-/root}"

VERSION="0.1.0"
REPO_URL="https://github.com/bolivian-peru/os-moda.git"
INSTALL_DIR="/opt/osmoda"
STATE_DIR="/var/lib/osmoda"
RUN_DIR="/run/osmoda"
OPENCLAW_DIR="/opt/openclaw"
GATEWAY_DIR="$INSTALL_DIR/packages/osmoda-gateway"
MCP_BRIDGE_DIR="$INSTALL_DIR/packages/osmoda-mcp-bridge"
WORKSPACE_DIR="/root/workspace"

# ---------------------------------------------------------------------------
# Colors
# ---------------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

log()   { echo -e "${GREEN}[osmoda]${NC} $*"; }
warn()  { echo -e "${YELLOW}[osmoda]${NC} $*"; }
error() { echo -e "${RED}[osmoda]${NC} $*" >&2; }
info()  { echo -e "${BLUE}[osmoda]${NC} $*"; }

die() { error "$@"; exit 1; }

# ---------------------------------------------------------------------------
# Container detection — fail fast with a helpful message
# ---------------------------------------------------------------------------
if [ -f /.dockerenv ] || grep -qsE '(docker|lxc|kubepods)' /proc/1/cgroup 2>/dev/null || [ "$(cat /proc/1/sched 2>/dev/null | head -1 | awk '{print $1}')" = "bash" ]; then
  die "osModa cannot run inside Docker, LXC, or containers.
  osModa is a full NixOS operating system — it needs a real VM or bare metal server.

  Supported environments:
    - Cloud VMs: Hetzner, DigitalOcean, AWS, GCP, Azure
    - Bare metal servers
    - QEMU/KVM virtual machines

  Easiest option: visit https://spawn.os.moda to deploy a managed osModa server."
fi

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
SKIP_NIXOS=false
API_KEY=""
BRANCH="main"
ORDER_ID=""
CALLBACK_URL=""
HEARTBEAT_SECRET=""
PROVIDER_TYPE=""
RUNTIME="claude-code"  # claude-code (default) or openclaw (legacy)
SNAPSHOT_MODE=false     # true when booting from pre-built NixOS snapshot
DEFAULT_MODEL=""       # initial default model for the osmoda agent
# Repeatable --credential flag. Each value: `label|provider|type|base64-secret`
CREDENTIALS=()

while [[ $# -gt 0 ]]; do
  case $1 in
    --skip-nixos)        SKIP_NIXOS=true; shift ;;
    --api-key)           API_KEY="$2"; shift 2 ;;
    --branch)            BRANCH="$2"; shift 2 ;;
    --order-id)          ORDER_ID="$2"; shift 2 ;;
    --callback-url)      CALLBACK_URL="$2"; shift 2 ;;
    --heartbeat-secret)  HEARTBEAT_SECRET="$2"; shift 2 ;;
    --provider)          PROVIDER_TYPE="$2"; shift 2 ;;
    --runtime)           RUNTIME="$2"; shift 2 ;;
    --snapshot)          SNAPSHOT_MODE=true; shift ;;
    --default-model)     DEFAULT_MODEL="$2"; shift 2 ;;
    --credential)        CREDENTIALS+=("$2"); shift 2 ;;
    --help|-h)
      echo "osModa Installer v${VERSION}"
      echo ""
      echo "Usage: curl -fsSL <url> | bash -s -- [options]"
      echo ""
      echo "Options:"
      echo "  --skip-nixos          Skip NixOS conversion (already on NixOS or Phase 2 post-reboot)"
      echo "  --api-key KEY         Set API key (base64-encoded or raw). Legacy; auto-migrates to a credential."
      echo "  --credential SPEC     Add a credential. Repeatable. Format:"
      echo "                          label|provider|type|base64-secret"
      echo "                        provider ∈ {anthropic, openai, openrouter}"
      echo "                        type     ∈ {oauth, api_key}"
      echo "  --default-model NAME  Initial default model (e.g. claude-opus-4-6)"
      echo "  --runtime NAME        Agent runtime: claude-code (default) or openclaw"
      echo "  --branch NAME         Git branch to install (default: main)"
      echo "  --order-id UUID       Spawn order ID (set by spawn.os.moda)"
      echo "  --callback-url URL    Heartbeat callback URL (set by spawn.os.moda)"
      echo "  --heartbeat-secret S  HMAC secret for heartbeat authentication"
      echo "  --provider TYPE       AI provider for --api-key fallback: anthropic or openai"
      echo "  --help                Show this help"
      exit 0
      ;;
    *) warn "Unknown option: $1"; shift ;;
  esac
done

# ---------------------------------------------------------------------------
# Banner
# ---------------------------------------------------------------------------
echo ""
echo -e "${BOLD}  ╔══════════════════════════════════════════╗${NC}"
echo -e "${BOLD}  ║         ${BLUE}osModa Installer v${VERSION}${NC}${BOLD}            ║${NC}"
echo -e "${BOLD}  ║   Your computer is about to get a brain. ║${NC}"
echo -e "${BOLD}  ╚══════════════════════════════════════════╝${NC}"
echo ""

# ---------------------------------------------------------------------------
# Progress reporting (sends live step updates to spawn.os.moda dashboard)
# ---------------------------------------------------------------------------
report_progress() {
  local step="$1" step_status="$2" detail="${3:-}"
  # Track the most-recently-started phase so the EXIT trap can report where
  # we died. Each new phase overwrites CURRENT_PHASE when status=started.
  if [ "$step_status" = "started" ]; then CURRENT_PHASE="$step"; fi
  if [ -z "${ORDER_ID:-}" ] || [ -z "${CALLBACK_URL:-}" ]; then return 0; fi
  local BASE_URL="${CALLBACK_URL%/api/heartbeat}"
  # Escape JSON string content: backslash, double-quote, newlines, tabs.
  # Without this, a literal `"` in a detail message (e.g. an error including a
  # command) breaks the JSON and the callback silently fails → dashboard
  # never shows the error. Replace order matters — backslashes first.
  local esc_step esc_status esc_detail
  esc_step=$(printf '%s' "$step" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')
  esc_status=$(printf '%s' "$step_status" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')
  esc_detail=$(printf '%s' "$detail" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e ':a;N;$!ba;s/\n/\\n/g' -e 's/\t/\\t/g')
  curl -sf --max-time 10 -X POST "$BASE_URL/api/provision-progress" \
    -H "Content-Type: application/json" \
    -H "X-Heartbeat-Secret: ${HEARTBEAT_SECRET:-}" \
    -d "{\"order_id\":\"$ORDER_ID\",\"step\":\"$esc_step\",\"status\":\"$esc_status\",\"detail\":\"$esc_detail\"}" \
    >/dev/null 2>&1 &
}

# Report a FATAL failure with context. Ships last 200 lines of the install log
# so the dashboard can render something actionable without an SSH round-trip.
report_failed() {
  local step="$1" reason="$2"
  if [ -z "${ORDER_ID:-}" ] || [ -z "${CALLBACK_URL:-}" ]; then return 0; fi
  local BASE_URL="${CALLBACK_URL%/api/heartbeat}"
  local log_file="/var/log/osmoda-cloud-init.log"
  local log_tail=""
  if [ -f "$log_file" ]; then
    log_tail=$(tail -n 200 "$log_file" 2>/dev/null | tr '\n' '\n' || true)
  fi
  # JSON-escape all three fields (same rules as report_progress).
  local esc_step esc_reason esc_tail
  esc_step=$(printf '%s' "$step" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g')
  esc_reason=$(printf '%s' "$reason" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e ':a;N;$!ba;s/\n/\\n/g')
  esc_tail=$(printf '%s' "$log_tail" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e ':a;N;$!ba;s/\n/\\n/g' -e 's/\t/\\t/g')
  curl -sf --max-time 15 -X POST "$BASE_URL/api/provision-failed" \
    -H "Content-Type: application/json" \
    -H "X-Heartbeat-Secret: ${HEARTBEAT_SECRET:-}" \
    -d "{\"order_id\":\"$ORDER_ID\",\"step\":\"$esc_step\",\"reason\":\"$esc_reason\",\"log_tail\":\"$esc_tail\"}" \
    >/dev/null 2>&1 || true
}

# Report errors on exit so dashboard shows failure. This fires on any non-zero
# exit path the trap sees — EXCEPT when the kernel reboots during nixos-infect
# or similar (SIGKILL → trap doesn't run). That's why the spawn watchdog flags
# stuck orders separately even when we send no callback here.
CURRENT_PHASE="preflight"
on_exit() {
  local rc=$?
  if [ $rc -ne 0 ]; then
    report_progress "$CURRENT_PHASE" "error" "Install exited with code $rc (phase: $CURRENT_PHASE, line $LINENO)"
    report_failed "$CURRENT_PHASE" "Install exited with code $rc at phase $CURRENT_PHASE"
    wait
  fi
}
trap on_exit EXIT

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------
report_progress "preflight" "started" "Running pre-flight checks"
log "Running pre-flight checks..."

if [ "$(id -u)" -ne 0 ]; then
  die "This installer must be run as root. Try: sudo bash"
fi

# Detect OS.
# /etc/NIXOS alone is unreliable: a failed nixos-infect leaves the marker
# behind even after the host reboots back to Ubuntu (we hit this on the
# CX22 test box on 2026-04-24). Require /run/current-system too — that
# symlink is created by the NixOS init and is only present when we're
# *actually* booted into NixOS.
if [ -f /etc/NIXOS ] && [ -L /run/current-system ]; then
  OS_TYPE="nixos"
  log "Detected: NixOS (running)"
  SKIP_NIXOS=true
elif [ -f /etc/os-release ]; then
  . /etc/os-release
  OS_TYPE="${ID:-unknown}"
  if [ -f /etc/NIXOS ] && [ ! -L /run/current-system ]; then
    warn "Stale /etc/NIXOS marker on $OS_TYPE host (failed nixos-infect?). Treating as ${PRETTY_NAME:-$OS_TYPE}."
  else
    log "Detected: ${PRETTY_NAME:-$OS_TYPE}"
  fi
else
  OS_TYPE="unknown"
  warn "Unknown OS. Proceeding anyway..."
fi

# Check architecture
ARCH=$(uname -m)
if [ "$ARCH" != "x86_64" ] && [ "$ARCH" != "aarch64" ]; then
  die "Unsupported architecture: $ARCH. osModa requires x86_64 or aarch64."
fi

log "Architecture: $ARCH"
log "Pre-flight checks passed."

# ---------------------------------------------------------------------------
# Fix password expiry — Hetzner Ubuntu 24.04 sets root password to expired,
# which makes PAM block SSH key auth ("Password change required but no TTY").
# Clear it so key-based SSH works immediately.
# ---------------------------------------------------------------------------
passwd -d root 2>/dev/null || true
chage -I -1 -m 0 -M 99999 -E -1 root 2>/dev/null || true

# ---------------------------------------------------------------------------
# Step 1: NixOS conversion (via nixos-infect)
# ---------------------------------------------------------------------------
# On Ubuntu/Debian: converts to NixOS, injects Phase 2 service into NixOS config,
# reboots into NixOS, Phase 2 downloads and runs install.sh --skip-nixos.
# On NixOS (or --skip-nixos): skips straight to daemon installation.
if [ "$SKIP_NIXOS" = false ]; then
  if [ "$OS_TYPE" = "ubuntu" ] || [ "$OS_TYPE" = "debian" ]; then
    report_progress "nixos" "started" "Converting to NixOS via nixos-infect"
    log "Step 1: Converting to NixOS..."
    log "This takes 8-15 minutes. Server reboots into NixOS, then daemons install automatically."

    # Auto-detect cloud provider
    PROVIDER="generic"
    if curl -sf -m 2 http://169.254.169.254/hetzner/v1/metadata >/dev/null 2>&1; then
      PROVIDER="hetznercloud"
    elif curl -sf -m 2 http://169.254.169.254/metadata/v1/ >/dev/null 2>&1; then
      PROVIDER="digitalocean"
    elif curl -sf -m 2 http://169.254.169.254/latest/meta-data/ >/dev/null 2>&1; then
      PROVIDER="ec2"
    fi
    log "Detected cloud provider: $PROVIDER"

    # Build Phase 2 args (these get baked into the NixOS config)
    PHASE2_ARGS="--skip-nixos"
    [ -n "$API_KEY" ] && PHASE2_ARGS="$PHASE2_ARGS --api-key $API_KEY"
    [ -n "$BRANCH" ] && PHASE2_ARGS="$PHASE2_ARGS --branch $BRANCH"
    [ -n "$ORDER_ID" ] && PHASE2_ARGS="$PHASE2_ARGS --order-id $ORDER_ID"
    [ -n "$CALLBACK_URL" ] && PHASE2_ARGS="$PHASE2_ARGS --callback-url $CALLBACK_URL"
    [ -n "$HEARTBEAT_SECRET" ] && PHASE2_ARGS="$PHASE2_ARGS --heartbeat-secret $HEARTBEAT_SECRET"
    [ -n "$PROVIDER_TYPE" ] && PHASE2_ARGS="$PHASE2_ARGS --provider $PROVIDER_TYPE"
    [ -n "$RUNTIME" ] && PHASE2_ARGS="$PHASE2_ARGS --runtime $(printf %q "$RUNTIME")"
    [ -n "$DEFAULT_MODEL" ] && PHASE2_ARGS="$PHASE2_ARGS --default-model $(printf %q "$DEFAULT_MODEL")"
    # Pass through every --credential in order. printf %q quotes anything the
    # downstream Phase-2 shell might split on (spaces, etc). Label sanitizer
    # restricts the charset, but defense in depth is cheap.
    for cred in "${CREDENTIALS[@]}"; do
      PHASE2_ARGS="$PHASE2_ARGS --credential $(printf %q "$cred")"
    done
    INSTALL_URL="https://raw.githubusercontent.com/bolivian-peru/os-moda/${BRANCH:-main}/scripts/install.sh"

    # Download nixos-infect and patch out its reboot so we can inject Phase 2 config
    log "Downloading nixos-infect..."
    curl -fsSL https://raw.githubusercontent.com/elitak/nixos-infect/master/nixos-infect > /tmp/nixos-infect.sh
    # Remove all reboot calls — we reboot manually after injecting Phase 2
    sed -i 's/reboot -f/echo "[osmoda] reboot deferred for Phase 2 injection"/g' /tmp/nixos-infect.sh
    sed -i 's/shutdown -r now/echo "[osmoda] shutdown deferred for Phase 2 injection"/g' /tmp/nixos-infect.sh
    # Fix nixos-infect bug: $bootFs can be empty on Hetzner, causing 'mv .bak' to fail
    sed -i 's/mv -v $bootFs $bootFs.bak/[ -n "$bootFs" ] \&\& mv -v $bootFs $bootFs.bak/g' /tmp/nixos-infect.sh
    sed -i 's/cp -a $bootFs $bootFs.bak/[ -n "$bootFs" ] \&\& cp -a $bootFs $bootFs.bak/g' /tmp/nixos-infect.sh
    sed -i 's/rm -rf $bootFs.bak/[ -n "$bootFs" ] \&\& rm -rf $bootFs.bak/g' /tmp/nixos-infect.sh

    log "Running nixos-infect (without reboot, 15 min timeout)..."
    report_progress "nixos" "started" "Running nixos-infect (5-10 min)"
    if timeout 900 bash -c 'NIX_CHANNEL=nixos-unstable PROVIDER="$1" bash /tmp/nixos-infect.sh' _ "$PROVIDER"; then
      log "nixos-infect complete. Injecting Phase 2 service into NixOS config..."
      report_progress "nixos" "started" "Injecting Phase 2 auto-install service"

      # Ensure configuration.nix declares pkgs in its function args
      if grep -q '{ \.\.\. }:' /etc/nixos/configuration.nix; then
        sed -i 's/{ \.\.\. }:/{ pkgs, ... }:/' /etc/nixos/configuration.nix
      elif ! grep -q 'pkgs' /etc/nixos/configuration.nix; then
        sed -i 's/{ config,/{ config, pkgs,/' /etc/nixos/configuration.nix 2>/dev/null || true
      fi

      # Preserve SSH keys through NixOS conversion (safety net — nixos-infect should handle this,
      # but losing SSH = losing the server, so we explicitly inject keys into NixOS config)
      SSH_KEYS_NIX=""
      if [ -f /root/.ssh/authorized_keys ]; then
        while IFS= read -r key; do
          [ -z "$key" ] && continue
          [[ "$key" == \#* ]] && continue
          # Escape double quotes in key
          escaped_key=$(echo "$key" | sed 's/"/\\"/g')
          SSH_KEYS_NIX="${SSH_KEYS_NIX}    \"${escaped_key}\"\n"
        done < /root/.ssh/authorized_keys
        log "Preserved $(grep -c '' /root/.ssh/authorized_keys) SSH keys for NixOS config"
      fi

      # Write Phase 2 NixOS config block to a temp file, then inject before closing brace
      cat > /tmp/osmoda-phase2.nix.fragment <<NIXEOF

  # osModa Phase 2: auto-install daemons after NixOS conversion
  environment.systemPackages = with pkgs; [ curl bash git cacert gcc gnumake pkg-config nix nodejs_22 ];

  # Fix root password (prevents SSH lockout after NixOS conversion)
  users.users.root.initialHashedPassword = "";
  users.mutableUsers = true;

  systemd.services.osmoda-phase2 = {
    description = "osModa Phase 2 Install (post-NixOS conversion)";
    after = [ "network-online.target" ];
    wants = [ "network-online.target" ];
    wantedBy = [ "multi-user.target" ];
    serviceConfig = {
      Type = "oneshot";
      RemainAfterExit = true;
      ExecStart = "/bin/sh -c 'export PATH=/run/current-system/sw/bin:\\\$PATH HOME=/root; curl -fsSL $INSTALL_URL | bash -s -- $PHASE2_ARGS; nixos-rebuild switch 2>/dev/null; systemctl disable osmoda-phase2.service 2>/dev/null'";
      TimeoutStartSec = 1800;
      Environment = [ "HOME=/root" "SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt" ];
    };
  };
NIXEOF
      # Insert fragment before the final closing brace of configuration.nix
      CONF="/etc/nixos/configuration.nix"
      FRAG=$(cat /tmp/osmoda-phase2.nix.fragment)
      # Replace final } with fragment + }
      awk -v frag="$FRAG" 'BEGIN{last=0} {lines[NR]=$0; last=NR} END{for(i=1;i<last;i++) print lines[i]; print frag; print lines[last]}' "$CONF" > "$CONF.tmp"
      mv "$CONF.tmp" "$CONF"
      rm -f /tmp/osmoda-phase2.nix.fragment
      log "Phase 2 service injected into /etc/nixos/configuration.nix"

      # Rebuild NixOS closure with Phase 2 service + password fix
      log "Rebuilding NixOS closure with Phase 2 service..."
      report_progress "nixos" "started" "Rebuilding NixOS closure (2-3 min)"
      # Source nix profile so nixos-rebuild is in PATH
      . /root/.nix-profile/etc/profile.d/nix.sh 2>/dev/null || . /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh 2>/dev/null || true
      export NIX_PATH="nixpkgs=/nix/var/nix/profiles/per-user/root/channels/nixos:nixos-config=/etc/nixos/configuration.nix"
      # Try nixos-rebuild first (works if nix channels are set up), fall back to nixos-install
      if nixos-rebuild switch 2>&1 | tail -5; then
        log "NixOS rebuilt with Phase 2. Rebooting into NixOS..."
      elif nixos-install --no-root-passwd 2>&1 | tail -5; then
        log "NixOS closure rebuilt via nixos-install. Rebooting..."
      else
        warn "NixOS rebuild failed — Phase 2 may not run after reboot."
        # Last resort: clear root password directly in shadow before reboot
        sed -i 's|^root:!:|root::|' /etc/shadow 2>/dev/null || true
        sed -i 's|^root:[^:]*:\([0-9]*\):[0-9]*:[0-9]*:[0-9]*:.*|root::\1:0:99999:7:::|' /etc/shadow 2>/dev/null || true
      fi
      report_progress "nixos" "done" "NixOS conversion complete"
      report_progress "reboot" "started" "Rebooting into NixOS (2-3 min)"
      reboot -f
      exit 0
    else
      error "nixos-infect failed. Falling through to install on current OS."
      warn "osModa will install on $OS_TYPE instead. NixOS conversion can be retried later."
      report_progress "nixos" "error" "nixos-infect failed, installing on $OS_TYPE"
    fi
  else
    warn "NixOS conversion not supported for $OS_TYPE. Installing on current OS."
  fi
fi

# If running as Phase 2 on NixOS, remove the Phase 2 service from configuration.nix
if [ "$OS_TYPE" = "nixos" ] && grep -q 'osmoda-phase2' /etc/nixos/configuration.nix 2>/dev/null; then
  report_progress "reboot" "done" "NixOS boot complete"
  log "Phase 2: Cleaning up auto-install service from NixOS config."
  CONF="/etc/nixos/configuration.nix"
  # Remove everything between "# osModa Phase 2" marker and its closing "};  };" block
  awk '/# osModa Phase 2/{skip=1} skip && /^  };$/{count++; if(count==2){skip=0; count=0; next}} !skip' "$CONF" > "$CONF.tmp"
  mv "$CONF.tmp" "$CONF"
  log "Phase 2: NixOS conversion complete, installing daemons..."
fi

# ---------------------------------------------------------------------------
# Snapshot Mode Fast-Path: Skip Steps 2-4 (deps, clone, build already done)
# ---------------------------------------------------------------------------
report_progress "preflight" "done" "$OS_TYPE $ARCH"

if [ "$SNAPSHOT_MODE" = true ] && [ -f "/opt/osmoda/target/release/agentd" ]; then
  log "Snapshot mode: Pre-compiled binaries detected, skipping build"
  INSTALL_DIR="/opt/osmoda"
  cd "$INSTALL_DIR"

  # Ensure bin symlinks exist
  mkdir -p "$INSTALL_DIR/bin"
  for binary in agentd agentctl osmoda-egress osmoda-keyd osmoda-watch osmoda-routines osmoda-voice osmoda-mesh osmoda-mcpd osmoda-teachd; do
    [ -f "target/release/$binary" ] && ln -sf "$INSTALL_DIR/target/release/$binary" "$INSTALL_DIR/bin/$binary"
  done
  export PATH="$INSTALL_DIR/bin:$PATH"

  # Pull latest source (templates, skills, install.sh changes)
  report_progress "clone" "started" "Updating source from GitHub"
  timeout 120 git fetch origin "${BRANCH:-main}" 2>/dev/null && git reset --hard "origin/${BRANCH:-main}" 2>/dev/null || true
  report_progress "clone" "done" "Source updated"

  # Check if Claude CLI needs update. Install from the gateway's pinned range
  # (see packages/osmoda-gateway/package.json) rather than @latest, so we don't
  # silently upgrade past the version the driver was tested against.
  GATEWAY_DIR="$INSTALL_DIR/packages/osmoda-gateway"
  MCP_BRIDGE_DIR="$INSTALL_DIR/packages/osmoda-mcp-bridge"
  CLAUDE_BIN="$GATEWAY_DIR/node_modules/.bin/claude"
  if [ ! -x "$CLAUDE_BIN" ]; then
    cd "$GATEWAY_DIR" && npm install --no-audit --no-fund 2>&1 | tail -3
  fi

  report_progress "build" "done" "Pre-compiled binaries (NixOS snapshot)"
  report_progress "dependencies" "done" "Snapshot mode — all deps pre-installed"

  # Jump directly to Step 5 (install agent runtime)
  # Steps 2-4 are completely skipped
else

# ---------------------------------------------------------------------------
# Step 2: Install dependencies (non-snapshot path)
# ---------------------------------------------------------------------------
report_progress "dependencies" "started" "Installing build tools + Node.js + Rust"
log "Step 2: Installing dependencies..."

# Ensure git is available
if ! command -v git &>/dev/null; then
  if command -v nix-env &>/dev/null; then
    nix-env -iA nixos.git
  elif command -v apt-get &>/dev/null; then
    apt-get update -qq && apt-get install -y -qq git
  fi
fi

# Ensure build tools for Rust
if [ "$OS_TYPE" = "nixos" ]; then
  # NixOS: install runtime tools if not already in PATH
  log "Installing NixOS runtime dependencies..."
  for pkg in jq; do
    if ! command -v "$pkg" &>/dev/null; then
      nix-env -iA "nixos.$pkg" 2>/dev/null || nix-env -iA "nixpkgs.$pkg" 2>/dev/null || true
    fi
  done
  USE_NIX_SHELL=true
elif [ "$OS_TYPE" = "ubuntu" ] || [ "$OS_TYPE" = "debian" ]; then
  log "Installing build dependencies for Ubuntu/Debian..."
  apt-get update -qq
  apt-get install -y -qq build-essential gcc g++ cmake pkg-config \
    libsqlite3-dev libssl-dev curl jq 2>&1 | tail -3
fi

# Ensure nix channels exist on NixOS (needed for nix-shell -p)
if [ "$OS_TYPE" = "nixos" ] && ! nix-instantiate --eval -E '<nixpkgs>' &>/dev/null; then
  log "Setting up nix channels (required for nix-shell)..."
  nix-channel --add https://nixos.org/channels/nixos-unstable nixos 2>/dev/null || true
  nix-channel --update 2>&1 | tail -2
fi

# Ensure Rust toolchain
if ! command -v cargo &>/dev/null; then
  if [ "$OS_TYPE" = "nixos" ]; then
    log "NixOS: Rust available via nix-shell (used during cargo build step)"
    # On NixOS, rustup doesn't work (dynamically linked). Use nix-shell instead.
    USE_NIX_SHELL=true
  else
    log "Installing Rust toolchain via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y 2>&1 | tail -3
  fi
fi
export PATH="$HOME/.cargo/bin:$PATH"

# Ensure Node.js for OpenClaw
if ! command -v node &>/dev/null; then
  if command -v nix-env &>/dev/null; then
    nix-env -iA nixos.nodejs_22 2>/dev/null || nix-env -iA nixpkgs.nodejs_22 2>/dev/null || true
  elif command -v apt-get &>/dev/null; then
    log "Installing Node.js 22 via NodeSource..."
    curl -fsSL https://deb.nodesource.com/setup_22.x | bash -
    apt-get install -y -qq nodejs
  fi
fi

log "Dependencies ready."

# ---------------------------------------------------------------------------
# Step 3: Clone/update the repo
# ---------------------------------------------------------------------------
report_progress "dependencies" "done" "Rust + Node.js + build tools ready"
report_progress "clone" "started" "Cloning osModa from GitHub"
log "Step 3: Getting osModa source..."

if [ -d "$INSTALL_DIR/.git" ]; then
  log "Updating existing installation..."
  cd "$INSTALL_DIR"
  timeout 120 git fetch origin "$BRANCH" || die "git fetch timed out (120s)"
  git reset --hard "origin/$BRANCH"
elif [ -d "$INSTALL_DIR" ]; then
  log "Removing stale installation at $INSTALL_DIR..."
  rm -rf "$INSTALL_DIR"
  timeout 300 git clone --branch "$BRANCH" --depth 1 "$REPO_URL" "$INSTALL_DIR" || die "git clone timed out (300s)"
  cd "$INSTALL_DIR"
else
  log "Cloning osModa..."
  timeout 300 git clone --branch "$BRANCH" --depth 1 "$REPO_URL" "$INSTALL_DIR" || die "git clone timed out (300s)"
  cd "$INSTALL_DIR"
fi

log "Source ready at $INSTALL_DIR"

# ---------------------------------------------------------------------------
# Step 4: Build Rust binaries
# ---------------------------------------------------------------------------
report_progress "clone" "done" "Source at $INSTALL_DIR"
report_progress "build" "started" "Compiling 9 Rust daemons (2-5 min)"
log "Step 4: Building all daemons (this takes 2-5 minutes on first build)..."

cd "$INSTALL_DIR"
BUILD_LOG=$(mktemp /tmp/osmoda-build-XXXXXX.log)
if [ "${USE_NIX_SHELL:-false}" = true ]; then
  # NixOS: need gcc + pkg-config for Rust builds with C dependencies
  if command -v gcc &>/dev/null && command -v pkg-config &>/dev/null; then
    # gcc + pkg-config already in PATH (e.g. NixOS snapshot with pre-installed tools)
    # Check if system Rust is new enough, otherwise use nix-shell with unstable
    if cargo build --release --workspace 2>&1 | tee "$BUILD_LOG"; then
      log "Build succeeded with system Rust."
    else
      warn "System Rust too old for some deps, retrying with nix-shell (nixpkgs-unstable)..."
      nix-channel --add https://nixos.org/channels/nixpkgs-unstable nixpkgs-unstable 2>/dev/null
      nix-channel --update 2>&1 | tail -2
      rm -f "$BUILD_LOG"
      BUILD_LOG=$(mktemp /tmp/osmoda-build-XXXXXX.log)
      if ! nix-shell -I nixpkgs=/nix/var/nix/profiles/per-user/root/channels/nixpkgs-unstable -p cargo rustc gcc pkg-config gnumake openssl sqlite openssl.dev sqlite.dev --run "cargo build --release --workspace" 2>&1 | tee "$BUILD_LOG"; then
        error "Build failed. Full output:"
        cat "$BUILD_LOG"
        rm -f "$BUILD_LOG"
        die "Cargo build failed. See errors above."
      fi
    fi
  else
    # Fallback: nix-shell provides gcc wrapper with proper C headers + linker paths
    log "Building inside nix-shell (gcc + pkg-config)..."
    # Ensure nix channels exist for nix-shell -p to work
    if ! nix-shell -p gcc pkg-config gnumake --run "echo ok" &>/dev/null; then
      log "Setting up nix channels..."
      nix-channel --add https://nixos.org/channels/nixos-24.11 nixos 2>/dev/null || true
      nix-channel --update 2>/dev/null || true
    fi
    if ! nix-shell -p gcc pkg-config gnumake --run "export PATH=\"\$HOME/.cargo/bin:\$PATH\" && cargo build --release --workspace" 2>&1 | tee "$BUILD_LOG"; then
      error "Build failed. Full output:"
      cat "$BUILD_LOG"
      rm -f "$BUILD_LOG"
      die "Cargo build failed inside nix-shell. See errors above."
    fi
  fi
else
  if ! cargo build --release --workspace 2>&1 | tee "$BUILD_LOG"; then
    error "Build failed. Full output:"
    cat "$BUILD_LOG"
    rm -f "$BUILD_LOG"
    die "Cargo build failed. See errors above."
  fi
fi
rm -f "$BUILD_LOG"

# Create bin directory and symlinks
mkdir -p "$INSTALL_DIR/bin"
MISSING_BINARIES=""
for binary in agentd agentctl osmoda-egress osmoda-keyd osmoda-watch osmoda-routines osmoda-voice osmoda-mesh osmoda-mcpd osmoda-teachd; do
  if [ -f "target/release/$binary" ]; then
    ln -sf "$INSTALL_DIR/target/release/$binary" "$INSTALL_DIR/bin/$binary"
    log "Built: $binary"
  else
    warn "Binary not found: $binary"
    MISSING_BINARIES="$MISSING_BINARIES $binary"
  fi
done

# Validate critical binaries exist
if [ ! -f "target/release/agentd" ]; then
  die "Critical binary missing: agentd. Build failed."
fi

# Add to PATH
if ! grep -q "osmoda/bin" /etc/profile.d/osmoda.sh 2>/dev/null; then
  mkdir -p /etc/profile.d
  echo "export PATH=\"$INSTALL_DIR/bin:\$PATH\"" > /etc/profile.d/osmoda.sh
fi
export PATH="$INSTALL_DIR/bin:$PATH"

log "Build complete."

fi  # end non-snapshot path (Steps 2-4)

# ---------------------------------------------------------------------------
# Step 5: Install agent runtime
# ---------------------------------------------------------------------------
report_progress "build" "done" "All daemons compiled"
report_progress "openclaw" "started" "Installing agent runtime ($RUNTIME)"
log "Step 5: Installing agent runtime ($RUNTIME)..."

if ! command -v npm &>/dev/null; then
  die "npm is required but not found. Install Node.js (>= 18) and retry."
fi

# v1.2+: osmoda-gateway is ALWAYS installed — it is the systemd unit, and
# it drives claude-code + openclaw as pluggable drivers (child processes).
# RUNTIME only determines what goes into agents.json as the default per-agent
# runtime AND whether we ALSO install the openclaw binary for that driver.

log "Installing gateway deps (includes Claude Code CLI pinned in package.json)..."
cd "$GATEWAY_DIR"
# Use plain `npm install` (respects package.json's pinned range — see
# @anthropic-ai/claude-code in optionalDependencies) rather than @latest, so
# the driver never gets a silently-upgraded CLI it wasn't tested against.
npm install --no-audit --no-fund 2>&1 | tail -3 || die "Failed to install gateway dependencies"
CLAUDE_BIN="$GATEWAY_DIR/node_modules/.bin/claude"
if [ ! -x "$CLAUDE_BIN" ]; then
  # optionalDependencies may skip on platform-incompatible installs; force-install
  # explicitly within the same pinned range rather than @latest.
  warn "Claude Code CLI missing after npm install — forcing install within pinned range"
  npm install --no-audit --no-fund --no-save '@anthropic-ai/claude-code@^2.1.75' 2>&1 | tail -3 || warn "Claude Code CLI install failed"
fi
if [ -x "$CLAUDE_BIN" ]; then
  ln -sf "$CLAUDE_BIN" /usr/local/bin/claude 2>/dev/null || true
  log "Claude Code CLI $(${CLAUDE_BIN} --version 2>/dev/null | head -1) installed"
else
  warn "Claude Code CLI not found at $CLAUDE_BIN — the claude-code driver will fail at spawn time"
fi

log "Building osmoda-gateway (modular, drivers: claude-code + openclaw)..."
cd "$GATEWAY_DIR"
if [ ! -f dist/index.js ] || [ "$(find src -newer dist/index.js 2>/dev/null | head -1)" ]; then
  npx tsc 2>/dev/null || die "Gateway TypeScript build failed"
fi

log "Installing MCP bridge (91 tools via MCP)..."
cd "$MCP_BRIDGE_DIR"
if [ ! -d node_modules ]; then
  npm install 2>&1 | tail -3 || die "Failed to install MCP bridge dependencies"
fi

# OpenClaw is optional — install it when the user explicitly chose runtime=openclaw
# so the openclaw driver has a binary to spawn. Users can switch runtime later
# via the dashboard without re-running install.sh; installing OpenClaw now just
# makes it available from day one.
if [ "$RUNTIME" = "openclaw" ]; then
  log "Installing OpenClaw binary (for the openclaw driver)..."
  if ! command -v openclaw &>/dev/null; then
    mkdir -p "$OPENCLAW_DIR"
    cd "$OPENCLAW_DIR"
    if [ ! -f package.json ]; then
      npm init -y >/dev/null 2>&1
    fi
    npm install openclaw 2>&1 | tail -3 || warn "Failed to install OpenClaw via npm — openclaw driver will be unavailable until resolved"
    if [ -x "$OPENCLAW_DIR/node_modules/.bin/openclaw" ]; then
      ln -sf "$OPENCLAW_DIR/node_modules/.bin/openclaw" /usr/local/bin/openclaw 2>/dev/null || true
      echo "export PATH=\"$OPENCLAW_DIR/node_modules/.bin:\$PATH\"" >> /etc/profile.d/osmoda.sh
      export PATH="$OPENCLAW_DIR/node_modules/.bin:$PATH"
      log "OpenClaw installed (version $(${OPENCLAW_DIR}/node_modules/.bin/openclaw --version 2>/dev/null | head -1 || echo '?'))"
    fi
  fi
fi

# ---------------------------------------------------------------------------
# Step 6: Set up tool bridge
# ---------------------------------------------------------------------------
report_progress "openclaw" "done" "$RUNTIME runtime installed"
report_progress "bridge" "started" "Setting up tool bridge (91 tools)"
log "Step 6: Setting up tool bridge..."

# MCP bridge is ALWAYS installed (both drivers route tools through it).
log "MCP bridge ready with 91 tools (via osmoda-mcp-bridge)."

# ALSO install the OpenClaw plugin flavor when OpenClaw is present, so the
# openclaw driver exposes the same 91 tools via OpenClaw's plugin system.
if [ "$RUNTIME" = "openclaw" ] || command -v openclaw &>/dev/null; then
  PLUGIN_SRC="$INSTALL_DIR/packages/osmoda-bridge"
  PLUGIN_DST="/root/.openclaw/extensions/osmoda-bridge"
  if [ -d "$PLUGIN_SRC" ]; then
    mkdir -p /root/.openclaw/extensions
    rm -rf "$PLUGIN_DST"
    cp -r "$PLUGIN_SRC" "$PLUGIN_DST"
    chown -R root:root "$PLUGIN_DST"
    log "OpenClaw plugin installed with 91 system tools."
  fi
fi

# ---------------------------------------------------------------------------
# Step 7: Multi-agent workspaces + skills (OpenClaw multi-agent routing)
# ---------------------------------------------------------------------------
report_progress "bridge" "done" "90 tools registered"
report_progress "workspaces" "started" "Setting up agent workspaces + skills"
log "Step 7: Setting up multi-agent workspaces..."

# Multi-agent workspace layout:
#   /root/workspace/               — main agent (Opus, full access) [shared]
#   ~/.openclaw/workspace-osmoda/  — OpenClaw main workspace (legacy)
#   ~/.openclaw/workspace-mobile/  — OpenClaw mobile workspace (legacy)
#   /var/lib/osmoda/workspace-mobile/ — mobile agent workspace (Claude Code)
OC_BASE="/root/.openclaw"
WS_OSMODA="$OC_BASE/workspace-osmoda"
WS_MOBILE="$OC_BASE/workspace-mobile"
WS_MOBILE_CC="$STATE_DIR/workspace-mobile"

mkdir -p "$WORKSPACE_DIR" "$WS_MOBILE_CC"
if [ "$RUNTIME" = "openclaw" ]; then
  mkdir -p "$WS_OSMODA" "$WS_MOBILE"
  mkdir -p "$OC_BASE/agents/osmoda/agent" "$OC_BASE/agents/osmoda/sessions"
  mkdir -p "$OC_BASE/agents/mobile/agent" "$OC_BASE/agents/mobile/sessions"
fi

# --- Main agent (osmoda): full templates + all skills ---
for tpl in AGENTS.md SOUL.md TOOLS.md IDENTITY.md USER.md HEARTBEAT.md; do
  if [ -f "$INSTALL_DIR/templates/$tpl" ]; then
    cp "$INSTALL_DIR/templates/$tpl" "$WORKSPACE_DIR/$tpl"
    [ "$RUNTIME" = "openclaw" ] && cp "$INSTALL_DIR/templates/$tpl" "$WS_OSMODA/$tpl"
  fi
done

if [ -d "$INSTALL_DIR/skills" ]; then
  mkdir -p "$WORKSPACE_DIR/skills"
  cp -r "$INSTALL_DIR/skills/"* "$WORKSPACE_DIR/skills/" 2>/dev/null || true
  if [ "$RUNTIME" = "openclaw" ]; then
    mkdir -p "$WS_OSMODA/skills"
    cp -r "$INSTALL_DIR/skills/"* "$WS_OSMODA/skills/" 2>/dev/null || true
  fi
fi

# --- Mobile agent: mobile-specific templates (concise style, full access) ---
if [ -d "$INSTALL_DIR/templates/agents/mobile" ]; then
  cp "$INSTALL_DIR/templates/agents/mobile/AGENTS.md" "$WS_MOBILE_CC/AGENTS.md"
  cp "$INSTALL_DIR/templates/agents/mobile/SOUL.md" "$WS_MOBILE_CC/SOUL.md"
  if [ "$RUNTIME" = "openclaw" ]; then
    cp "$INSTALL_DIR/templates/agents/mobile/AGENTS.md" "$WS_MOBILE/AGENTS.md"
    cp "$INSTALL_DIR/templates/agents/mobile/SOUL.md" "$WS_MOBILE/SOUL.md"
  fi
fi
# Share TOOLS.md and IDENTITY.md from main templates
for tpl in TOOLS.md IDENTITY.md USER.md; do
  if [ -f "$INSTALL_DIR/templates/$tpl" ]; then
    cp "$INSTALL_DIR/templates/$tpl" "$WS_MOBILE_CC/$tpl"
    [ "$RUNTIME" = "openclaw" ] && cp "$INSTALL_DIR/templates/$tpl" "$WS_MOBILE/$tpl"
  fi
done

# Mobile skills: all skills (same as main agent)
MOBILE_SKILLS="self-healing morning-briefing security-hardening natural-language-config predictive-resources drift-detection generation-timeline flight-recorder nix-optimizer system-monitor system-packages system-config file-manager network-manager service-explorer app-deployer deploy-ai-agent swarm-predict scaled-swarm-predict"
if [ -d "$INSTALL_DIR/skills" ]; then
  mkdir -p "$WS_MOBILE_CC/skills"
  [ "$RUNTIME" = "openclaw" ] && mkdir -p "$WS_MOBILE/skills"
  for skill in $MOBILE_SKILLS; do
    if [ -d "$INSTALL_DIR/skills/$skill" ]; then
      cp -r "$INSTALL_DIR/skills/$skill" "$WS_MOBILE_CC/skills/$skill"
      [ "$RUNTIME" = "openclaw" ] && cp -r "$INSTALL_DIR/skills/$skill" "$WS_MOBILE/skills/$skill"
    fi
  done
fi

# Create state directories with secure permissions
mkdir -p "$STATE_DIR"/{memory,ledger,config,keyd/keys,watch,routines,mesh,mcp,teachd,apps,swarm}
mkdir -p "$RUN_DIR"
mkdir -p /var/backups/osmoda
chmod 700 "$STATE_DIR/config"
chmod 700 "$STATE_DIR/keyd"
chmod 700 "$STATE_DIR/keyd/keys"
chmod 700 "$STATE_DIR/mesh"

log "Multi-agent workspaces installed (osmoda + mobile)."

# Store spawn config (if provisioned via spawn.os.moda)
if [ -n "$ORDER_ID" ]; then
  printf '%s\n' "$ORDER_ID" > "$STATE_DIR/config/order-id"
  chmod 600 "$STATE_DIR/config/order-id"
  log "Spawn order ID stored."
  if [ -n "$CALLBACK_URL" ]; then
    printf '%s\n' "$CALLBACK_URL" > "$STATE_DIR/config/callback-url"
    chmod 600 "$STATE_DIR/config/callback-url"
  fi
  if [ -n "$HEARTBEAT_SECRET" ]; then
    printf '%s\n' "$HEARTBEAT_SECRET" > "$STATE_DIR/config/heartbeat-secret"
    chmod 600 "$STATE_DIR/config/heartbeat-secret"
  fi
fi

# Store runtime choice
printf '%s\n' "$RUNTIME" > "$STATE_DIR/config/runtime"
chmod 644 "$STATE_DIR/config/runtime"

# ---------------------------------------------------------------------------
# Step 8: Set up API key (if provided) or generate placeholder config
# ---------------------------------------------------------------------------
report_progress "workspaces" "done" "osmoda + mobile agents configured"
if [ -n "$API_KEY" ]; then
  report_progress "apikey" "started" "Configuring API key + multi-agent auth"
  log "Step 8: Configuring API key..."

  # Decode base64 API key if it looks base64-encoded (from spawn cloud-init)
  DECODED_KEY="$API_KEY"
  if echo "$API_KEY" | grep -qE '^[A-Za-z0-9+/=]{20,}$' && ! echo "$API_KEY" | grep -q '^sk-'; then
    DECODED_KEY=$(echo "$API_KEY" | base64 -d 2>/dev/null || echo "$API_KEY")
  fi

  # Determine provider
  EFFECTIVE_PROVIDER="${PROVIDER_TYPE:-anthropic}"

  printf '%s\n' "$DECODED_KEY" > "$STATE_DIR/config/api-key"
  chmod 600 "$STATE_DIR/config/api-key"

  if [ "$EFFECTIVE_PROVIDER" = "openai" ]; then
    printf 'OPENAI_API_KEY=%s\n' "$DECODED_KEY" > "$STATE_DIR/config/env"
  elif echo "$DECODED_KEY" | grep -q '^sk-ant-oat'; then
    # OAuth token — use CLAUDE_CODE_OAUTH_TOKEN (subscription credits via Claude Code)
    printf 'CLAUDE_CODE_OAUTH_TOKEN=%s\n' "$DECODED_KEY" > "$STATE_DIR/config/env"
  else
    # Console API key — use ANTHROPIC_API_KEY
    printf 'ANTHROPIC_API_KEY=%s\n' "$DECODED_KEY" > "$STATE_DIR/config/env"
  fi
  chmod 600 "$STATE_DIR/config/env"

  # Write auth profiles (OpenClaw) or gateway config (Claude Code)
  if [ "$RUNTIME" = "claude-code" ]; then
    # Claude Code: generate gateway.json (no auth-profiles needed)
    GATEWAY_TOKEN=$(openssl rand -hex 32 2>/dev/null || head -c 32 /dev/urandom | xxd -p -c 64)
    printf '%s' "$GATEWAY_TOKEN" > "$STATE_DIR/config/gateway-token"
    chmod 600 "$STATE_DIR/config/gateway-token"

    if command -v node &>/dev/null; then
      node - "$GATEWAY_TOKEN" <<'GWCONFIGEOF'
const fs = require('fs');
const gwToken = process.argv[2] || '';
const config = {
  port: 18789,
  authToken: gwToken,
  agents: [
    { id: 'osmoda', model: 'claude-opus-4-6', "default": true, systemPromptFile: '/root/workspace/SOUL.md' },
    { id: 'mobile', model: 'claude-sonnet-4-6', systemPromptFile: '/var/lib/osmoda/workspace-mobile/SOUL.md' }
  ],
  bindings: [
    { agentId: 'mobile', channel: 'telegram' },
    { agentId: 'mobile', channel: 'whatsapp' }
  ],
  mcpBridgePath: '/opt/osmoda/packages/osmoda-mcp-bridge/dist/index.js'
};
fs.mkdirSync('/var/lib/osmoda/config', { recursive: true });
fs.writeFileSync('/var/lib/osmoda/config/gateway.json', JSON.stringify(config, null, 2));
GWCONFIGEOF
      log "Gateway config written to $STATE_DIR/config/gateway.json"
    fi
  fi

  # Write OpenClaw auth-profiles.json for BOTH agents (shared API key) — only for openclaw runtime
  if [ "$RUNTIME" = "openclaw" ]; then
  for AGENT_ID in osmoda mobile; do
    mkdir -p "/root/.openclaw/agents/$AGENT_ID/agent"
    if command -v node &>/dev/null; then
      if [ "$EFFECTIVE_PROVIDER" = "openai" ]; then
        node - "$DECODED_KEY" "$AGENT_ID" <<'AUTHEOF'
const fs = require('fs');
const key = process.argv[2];
const agentId = process.argv[3];
const auth = { type: 'api_key', provider: 'openai', key: key };
fs.writeFileSync('/root/.openclaw/agents/' + agentId + '/agent/auth-profiles.json', JSON.stringify(auth, null, 2));
AUTHEOF
      else
        node - "$DECODED_KEY" "$AGENT_ID" <<'AUTHEOF'
const fs = require('fs');
const key = process.argv[2];
const agentId = process.argv[3];
const isOAuth = key.startsWith('sk-ant-oat');
const auth = isOAuth
  ? { type: 'token', provider: 'anthropic', token: key }
  : { type: 'api_key', provider: 'anthropic', key: key };
fs.writeFileSync('/root/.openclaw/agents/' + agentId + '/agent/auth-profiles.json', JSON.stringify(auth, null, 2));
AUTHEOF
      fi
    fi
  done
  fi  # end RUNTIME=openclaw auth-profiles block

  # Generate gateway token for WS relay auth (needed by both runtimes)
  if [ ! -f "$STATE_DIR/config/gateway-token" ]; then
  GATEWAY_TOKEN=$(openssl rand -hex 32 2>/dev/null || head -c 32 /dev/urandom | xxd -p -c 64)
  printf '%s' "$GATEWAY_TOKEN" > "$STATE_DIR/config/gateway-token"
  chmod 600 "$STATE_DIR/config/gateway-token"
  log "Generated gateway token for relay auth"
  fi  # end gateway-token generation

  # ------------------------------------------------------------------
  # Modular bootstrap: agents.json + bootstrap-credentials.json
  # ------------------------------------------------------------------
  # The gateway (v1.2+) reads these on first boot. bootstrap-credentials.json
  # is plaintext JSON that the gateway imports into its encrypted store and
  # then deletes. agents.json is the durable per-server config that users
  # can edit from the dashboard.
  #
  # This block runs regardless of runtime choice — the gateway treats them
  # the same way; only the runtime field differs.
  if command -v node &>/dev/null; then
    export OSMODA_INSTALL_API_KEY="$DECODED_KEY"
    export OSMODA_INSTALL_PROVIDER="$EFFECTIVE_PROVIDER"
    export OSMODA_INSTALL_RUNTIME="$RUNTIME"
    export OSMODA_INSTALL_DEFAULT_MODEL="$DEFAULT_MODEL"
    # Stash --credential specs as JSON array (base64 to survive shell boundaries)
    CREDS_JSON="[]"
    if [ "${#CREDENTIALS[@]}" -gt 0 ]; then
      # Build a JSON array of raw strings — gateway bootstrap parses each "label|provider|type|b64secret"
      CREDS_JSON=$(printf '%s\n' "${CREDENTIALS[@]}" | node -e '
        let buf = "";
        process.stdin.on("data", c => buf += c);
        process.stdin.on("end", () => {
          const arr = buf.split("\n").filter(Boolean);
          process.stdout.write(JSON.stringify(arr));
        });
      ')
    fi
    export OSMODA_INSTALL_CREDENTIALS="$CREDS_JSON"

    node <<'BOOTSTRAPEOF'
const fs = require("fs");
const path = require("path");
const CFG = "/var/lib/osmoda/config";
fs.mkdirSync(CFG, { recursive: true, mode: 0o700 });

const apiKey = process.env.OSMODA_INSTALL_API_KEY || "";
const provider = process.env.OSMODA_INSTALL_PROVIDER || "anthropic";
const runtime = process.env.OSMODA_INSTALL_RUNTIME || "claude-code";
const defaultModel = process.env.OSMODA_INSTALL_DEFAULT_MODEL || "claude-opus-4-6";
const credSpecs = JSON.parse(process.env.OSMODA_INSTALL_CREDENTIALS || "[]");

const creds = [];
let defaultCredId = null;

function pushCred({ label, provider, type, secret }) {
  if (!secret || secret.length < 10) return null;
  const id = "cred_" + require("crypto").randomBytes(12).toString("hex");
  const now = new Date().toISOString();
  creds.push({ id, label, provider, type, secret, created_at: now });
  if (!defaultCredId) defaultCredId = id;
  return id;
}

// Legacy --api-key → credential
if (apiKey) {
  const type = apiKey.startsWith("sk-ant-oat") ? "oauth" : "api_key";
  pushCred({
    label: provider === "openai" ? "OpenAI API key" : (type === "oauth" ? "Claude OAuth" : "Anthropic API key"),
    provider,
    type,
    secret: apiKey,
  });
}

// --credential label|provider|type|base64-secret (repeatable, wins in order)
for (const spec of credSpecs) {
  const parts = spec.split("|");
  if (parts.length < 4) { console.warn("[bootstrap] skipping malformed credential spec"); continue; }
  const [label, cprovider, ctype, b64] = parts;
  let secret;
  try { secret = Buffer.from(b64, "base64").toString("utf8"); }
  catch { console.warn("[bootstrap] skipping credential with bad base64"); continue; }
  pushCred({ label, provider: cprovider, type: ctype, secret });
}

// Write bootstrap file for gateway to absorb on first boot.
if (creds.length > 0) {
  fs.writeFileSync(
    path.join(CFG, "bootstrap-credentials.json"),
    JSON.stringify({ version: 1, default_credential_id: defaultCredId, credentials: creds }, null, 2),
    { mode: 0o600 },
  );
}

// Write agents.json pointing to the default credential.
const now = new Date().toISOString();
const mkAgent = (id, displayName, model, channels) => ({
  id,
  display_name: displayName,
  runtime,
  credential_id: defaultCredId || "",
  model,
  channels,
  profile_dir: "/var/lib/osmoda/workspace-" + id,
  enabled: Boolean(defaultCredId),
  updated_at: now,
});

const agents = [
  mkAgent("osmoda", "osModa (full access)", defaultModel || "claude-opus-4-6", ["web", "api"]),
  mkAgent("mobile", "osModa mobile", "claude-sonnet-4-6", ["telegram", "whatsapp"]),
];

const agentsFile = {
  version: 1,
  agents,
  bindings: [
    { channel: "telegram", agent_id: "mobile" },
    { channel: "whatsapp", agent_id: "mobile" },
  ],
};
fs.writeFileSync(path.join(CFG, "agents.json"), JSON.stringify(agentsFile, null, 2), { mode: 0o640 });
console.log("[bootstrap] wrote agents.json (" + agents.length + " agents, runtime=" + runtime + ", default_cred=" + (defaultCredId || "<none>") + ")");
BOOTSTRAPEOF
    unset OSMODA_INSTALL_API_KEY OSMODA_INSTALL_PROVIDER OSMODA_INSTALL_RUNTIME OSMODA_INSTALL_DEFAULT_MODEL OSMODA_INSTALL_CREDENTIALS
  fi
  # end modular bootstrap

  # Generate multi-agent OpenClaw config with env block + compaction (openclaw only)
  if [ "$RUNTIME" = "openclaw" ] && command -v node &>/dev/null; then
    node - "$GATEWAY_TOKEN" "$DECODED_KEY" "$EFFECTIVE_PROVIDER" <<'CONFIGEOF'
const fs = require('fs');
const gwToken = process.argv[2] || '';
const apiKey = process.argv[3] || '';
const provider = process.argv[4] || 'anthropic';

// Build env block: OAuth tokens go in BOTH CLAUDE_CODE_OAUTH_TOKEN and ANTHROPIC_API_KEY
const env = {};
if (apiKey && provider === 'anthropic') {
  env.ANTHROPIC_API_KEY = apiKey;
  if (apiKey.startsWith('sk-ant-oat')) {
    env.CLAUDE_CODE_OAUTH_TOKEN = apiKey;
  }
} else if (apiKey && provider === 'openai') {
  env.OPENAI_API_KEY = apiKey;
}

const config = {
  env: env,
  gateway: { mode: 'local', auth: gwToken ? { mode: 'token', token: gwToken } : { mode: 'none' } },
  plugins: { allow: ['osmoda-bridge', 'device-pair', 'memory-core', 'phone-control', 'talk-voice'] },
  agents: {
    defaults: {
      compaction: { mode: 'safeguard' }
    },
    list: [
      {
        id: 'osmoda',
        default: true,
        name: 'osModa',
        workspace: '/root/.openclaw/workspace-osmoda',
        agentDir: '/root/.openclaw/agents/osmoda/agent',
        model: 'anthropic/claude-opus-4-6'
      },
      {
        id: 'mobile',
        name: 'osModa Mobile',
        workspace: '/root/.openclaw/workspace-mobile',
        agentDir: '/root/.openclaw/agents/mobile/agent',
        model: 'anthropic/claude-sonnet-4-6'
      }
    ]
  },
  bindings: [
    { agentId: 'mobile', match: { channel: 'telegram' } },
    { agentId: 'mobile', match: { channel: 'whatsapp' } }
  ]
};
fs.writeFileSync('/root/.openclaw/openclaw.json', JSON.stringify(config, null, 2));
CONFIGEOF
    log "Multi-agent config written to /root/.openclaw/openclaw.json"
  fi

  # Gateway env vars for daemon sockets
  cat > "$STATE_DIR/config/gateway-env" <<GWEOF
OSMODA_SOCKET=/run/osmoda/agentd.sock
OSMODA_KEYD_SOCKET=/run/osmoda/keyd.sock
OSMODA_WATCH_SOCKET=/run/osmoda/watch.sock
OSMODA_ROUTINES_SOCKET=/run/osmoda/routines.sock
OSMODA_VOICE_SOCKET=/run/osmoda/voice.sock
OSMODA_MESH_SOCKET=/run/osmoda/mesh.sock
OSMODA_MCPD_SOCKET=/run/osmoda/mcpd.sock
OSMODA_TEACHD_SOCKET=/run/osmoda/teachd.sock
GWEOF
  chmod 600 "$STATE_DIR/config/gateway-env"

  log "API key configured for both agents."
else
  log "Step 8: No API key provided — generating gateway config (key can be set via dashboard)."

  # Still need gateway token even without API key
  GATEWAY_TOKEN=$(openssl rand -hex 32 2>/dev/null || head -c 32 /dev/urandom | xxd -p -c 64)
  printf '%s' "$GATEWAY_TOKEN" > "$STATE_DIR/config/gateway-token"
  chmod 600 "$STATE_DIR/config/gateway-token"

  if [ "$RUNTIME" = "claude-code" ]; then
    # Claude Code: write gateway.json (no API key, gateway won't start until key is set)
    if command -v node &>/dev/null; then
      node - "$GATEWAY_TOKEN" <<'GWCONFIGEOF'
const fs = require('fs');
const gwToken = process.argv[2] || '';
const config = {
  port: 18789,
  authToken: gwToken,
  agents: [
    { id: 'osmoda', model: 'claude-opus-4-6', "default": true, systemPromptFile: '/root/workspace/SOUL.md' },
    { id: 'mobile', model: 'claude-sonnet-4-6', systemPromptFile: '/var/lib/osmoda/workspace-mobile/SOUL.md' }
  ],
  bindings: [
    { agentId: 'mobile', channel: 'telegram' },
    { agentId: 'mobile', channel: 'whatsapp' }
  ],
  mcpBridgePath: '/opt/osmoda/packages/osmoda-mcp-bridge/dist/index.js'
};
fs.mkdirSync('/var/lib/osmoda/config', { recursive: true });
fs.writeFileSync('/var/lib/osmoda/config/gateway.json', JSON.stringify(config, null, 2));
GWCONFIGEOF
      log "Gateway config written (no API key — set via dashboard)"
    fi
  else
    # OpenClaw: write openclaw.json
    if command -v node &>/dev/null; then
      node - "$GATEWAY_TOKEN" <<'CONFIGEOF'
const fs = require('fs');
const gwToken = process.argv[2] || '';
const config = {
  env: {},
  gateway: { mode: 'local', auth: gwToken ? { mode: 'token', token: gwToken } : { mode: 'none' } },
  plugins: { allow: ['osmoda-bridge', 'device-pair', 'memory-core', 'phone-control', 'talk-voice'] },
  agents: {
    defaults: { compaction: { mode: 'safeguard' } },
    list: [
      { id: 'osmoda', default: true, name: 'osModa', workspace: '/root/.openclaw/workspace-osmoda', agentDir: '/root/.openclaw/agents/osmoda/agent', model: 'anthropic/claude-opus-4-6' },
      { id: 'mobile', name: 'osModa Mobile', workspace: '/root/.openclaw/workspace-mobile', agentDir: '/root/.openclaw/agents/mobile/agent', model: 'anthropic/claude-sonnet-4-6' }
    ]
  },
  bindings: [ { agentId: 'mobile', match: { channel: 'telegram' } }, { agentId: 'mobile', match: { channel: 'whatsapp' } } ]
};
fs.writeFileSync('/root/.openclaw/openclaw.json', JSON.stringify(config, null, 2));
CONFIGEOF
      log "Multi-agent config written (no API key — set via dashboard)"
    fi
  fi

  # Gateway env vars for daemon sockets
  cat > "$STATE_DIR/config/gateway-env" <<GWEOF
OSMODA_SOCKET=/run/osmoda/agentd.sock
OSMODA_KEYD_SOCKET=/run/osmoda/keyd.sock
OSMODA_WATCH_SOCKET=/run/osmoda/watch.sock
OSMODA_ROUTINES_SOCKET=/run/osmoda/routines.sock
OSMODA_VOICE_SOCKET=/run/osmoda/voice.sock
OSMODA_MESH_SOCKET=/run/osmoda/mesh.sock
OSMODA_MCPD_SOCKET=/run/osmoda/mcpd.sock
OSMODA_TEACHD_SOCKET=/run/osmoda/teachd.sock
GWEOF
  chmod 600 "$STATE_DIR/config/gateway-env"

  # Create placeholder auth-profiles for openclaw agents (user will set key later)
  if [ "$RUNTIME" = "openclaw" ]; then
    for AGENT_ID in osmoda mobile; do
      mkdir -p "/root/.openclaw/agents/$AGENT_ID/agent"
      cat > "/root/.openclaw/agents/$AGENT_ID/agent/auth-profiles.json" <<'AUTHEOF'
{"type":"api_key","provider":"anthropic","key":""}
AUTHEOF
    done
    log "Placeholder auth-profiles created (key will be set by user)."
  fi
fi

# ---------------------------------------------------------------------------
# Step 8b: Generate device identity keypair for WS relay
# ---------------------------------------------------------------------------
# The WS relay needs a device identity for authenticating with the spawn server.
if [ "$RUNTIME" = "claude-code" ]; then
  IDENTITY_DIR="$STATE_DIR/identity"
else
  IDENTITY_DIR="/root/.openclaw/identity"
fi
if [ ! -f "$IDENTITY_DIR/device.json" ]; then
  log "Generating Ed25519 device identity keypair..."
  mkdir -p "$IDENTITY_DIR"
  node -e "
const crypto=require('crypto'),fs=require('fs'),path=require('path');
const{publicKey,privateKey}=crypto.generateKeyPairSync('ed25519');
const spkiDer=publicKey.export({type:'spki',format:'der'});
const rawKey=spkiDer.subarray(spkiDer.length-32);
const deviceId=crypto.createHash('sha256').update(rawKey).digest('hex');
fs.writeFileSync('$IDENTITY_DIR/device.json',JSON.stringify({
  version:1,deviceId,
  publicKeyPem:publicKey.export({type:'spki',format:'pem'}),
  privateKeyPem:privateKey.export({type:'pkcs8',format:'pem'}),
  createdAtMs:Date.now()
},null,2),{mode:0o600});
console.log('[device] keypair generated, deviceId:',deviceId.slice(0,16)+'...');
" 2>&1 || warn "Device keypair generation failed"
else
  log "Device identity keypair already exists."
fi

# ---------------------------------------------------------------------------
# Step 9: Create and start systemd services
# ---------------------------------------------------------------------------
report_progress "apikey" "done" "Auth profiles written"
report_progress "services" "started" "Starting 9 daemons + gateway"
log "Step 9: Starting services..."

if [ "$OS_TYPE" = "nixos" ]; then
  # On NixOS, the recommended path is the osmoda.nix NixOS module.
  # But for install.sh bootstrap (e.g. fresh cloud server), write real systemd
  # unit files to /etc/systemd/system/ — they work on NixOS alongside the module.
  log "NixOS detected. Writing systemd unit files for persistent daemon management..."
  log "For production use, prefer: services.osmoda.enable = true in configuration.nix"
fi

SKIP_SYSTEMD=false
if [ "$OS_TYPE" = "nixos" ] && [ ! -w "/etc/systemd/system" ]; then
  # NixOS: /etc/systemd/system is read-only (Nix store). Use runtime directory.
  SYSTEMD_DIR="/run/systemd/system"
  mkdir -p "$SYSTEMD_DIR" 2>/dev/null || true
  MKDIR_BIN="/run/current-system/sw/bin/mkdir"
  log "NixOS read-only /etc: using $SYSTEMD_DIR for service units"
else
  SYSTEMD_DIR="/etc/systemd/system"
  MKDIR_BIN="/bin/mkdir"
fi

mkdir -p "$RUN_DIR" "$STATE_DIR"
mkdir -p "$STATE_DIR"/{keyd/keys,watch,routines,mesh,config}
chmod 700 "$STATE_DIR/keyd" "$STATE_DIR/keyd/keys" "$STATE_DIR/mesh"

# Detect public IP for mesh daemon (used in invite codes so peers can reach us)
PUBLIC_IP=""
if curl -sf -m 3 http://169.254.169.254/hetzner/v1/metadata/public-ipv4 >/dev/null 2>&1; then
  PUBLIC_IP=$(curl -sf -m 3 http://169.254.169.254/hetzner/v1/metadata/public-ipv4 2>/dev/null)
elif curl -sf -m 3 http://169.254.169.254/metadata/v1/interfaces/public/0/ipv4/address >/dev/null 2>&1; then
  PUBLIC_IP=$(curl -sf -m 3 http://169.254.169.254/metadata/v1/interfaces/public/0/ipv4/address 2>/dev/null)
fi
if [ -z "$PUBLIC_IP" ]; then
  PUBLIC_IP=$(curl -sf -m 3 https://ifconfig.me 2>/dev/null || hostname -I | awk '{print $1}')
fi
log "Detected public IP for mesh: ${PUBLIC_IP:-unknown}"

if [ "$SKIP_SYSTEMD" = false ]; then
# agentd service
cat > "$SYSTEMD_DIR/osmoda-agentd.service" <<EOF
[Unit]
Description=osModa System Daemon
After=network.target

[Service]
Type=simple
ExecStart=$INSTALL_DIR/bin/agentd --socket $RUN_DIR/agentd.sock --state-dir $STATE_DIR
Restart=always
RestartSec=5
Environment=RUST_LOG=info
ExecStartPre=+${MKDIR_BIN} -p $RUN_DIR
ExecStartPre=+${MKDIR_BIN} -p $STATE_DIR

[Install]
WantedBy=multi-user.target
EOF

# Agent gateway service (runtime-dependent)
if [ "$RUNTIME" = "claude-code" ]; then
cat > "$SYSTEMD_DIR/osmoda-gateway.service" <<EOF
[Unit]
Description=osModa Gateway (Claude Code SDK)
After=network.target osmoda-agentd.service
Wants=osmoda-agentd.service

[Service]
Type=simple
ExecStart=$(which node) $GATEWAY_DIR/dist/index.js
Restart=always
RestartSec=5
WorkingDirectory=/root
EnvironmentFile=-$STATE_DIR/config/env
Environment=HOME=/root
Environment=NODE_ENV=production
Environment=OSMODA_GATEWAY_CONFIG=$STATE_DIR/config/gateway.json
Environment=OSMODA_SOCKET=$RUN_DIR/agentd.sock
Environment=OSMODA_KEYD_SOCKET=$RUN_DIR/keyd.sock
Environment=OSMODA_WATCH_SOCKET=$RUN_DIR/watch.sock
Environment=OSMODA_ROUTINES_SOCKET=$RUN_DIR/routines.sock
Environment=OSMODA_VOICE_SOCKET=$RUN_DIR/voice.sock
Environment=OSMODA_MESH_SOCKET=$RUN_DIR/mesh.sock
Environment=OSMODA_MCPD_SOCKET=$RUN_DIR/mcpd.sock
Environment=OSMODA_TEACHD_SOCKET=$RUN_DIR/teachd.sock

[Install]
WantedBy=multi-user.target
EOF
else
cat > "$SYSTEMD_DIR/osmoda-gateway.service" <<EOF
[Unit]
Description=osModa Gateway (OpenClaw)
After=network.target osmoda-agentd.service
Wants=osmoda-agentd.service

[Service]
Type=simple
ExecStart=$OPENCLAW_DIR/node_modules/.bin/openclaw gateway
Restart=always
RestartSec=5
WorkingDirectory=$WORKSPACE_DIR
EnvironmentFile=-$STATE_DIR/config/env
Environment=HOME=/root
Environment=NODE_ENV=production
Environment=OSMODA_SOCKET=$RUN_DIR/agentd.sock
Environment=OSMODA_KEYD_SOCKET=$RUN_DIR/keyd.sock
Environment=OSMODA_WATCH_SOCKET=$RUN_DIR/watch.sock
Environment=OSMODA_ROUTINES_SOCKET=$RUN_DIR/routines.sock
Environment=OSMODA_VOICE_SOCKET=$RUN_DIR/voice.sock
Environment=OSMODA_MESH_SOCKET=$RUN_DIR/mesh.sock
Environment=OSMODA_MCPD_SOCKET=$RUN_DIR/mcpd.sock
Environment=OSMODA_TEACHD_SOCKET=$RUN_DIR/teachd.sock

[Install]
WantedBy=multi-user.target
EOF
fi

# keyd
cat > "$SYSTEMD_DIR/osmoda-keyd.service" <<EOF
[Unit]
Description=osModa Crypto Wallet Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=$INSTALL_DIR/bin/osmoda-keyd --socket $RUN_DIR/keyd.sock --data-dir $STATE_DIR/keyd --policy-file $STATE_DIR/keyd/policy.json --agentd-socket $RUN_DIR/agentd.sock
Restart=always
RestartSec=5
Environment=RUST_LOG=info
PrivateNetwork=true

[Install]
WantedBy=multi-user.target
EOF

# watch
cat > "$SYSTEMD_DIR/osmoda-watch.service" <<EOF
[Unit]
Description=osModa SafeSwitch Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=$INSTALL_DIR/bin/osmoda-watch --socket $RUN_DIR/watch.sock --agentd-socket $RUN_DIR/agentd.sock --data-dir $STATE_DIR/watch
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

# routines
cat > "$SYSTEMD_DIR/osmoda-routines.service" <<EOF
[Unit]
Description=osModa Routines Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=$INSTALL_DIR/bin/osmoda-routines --socket $RUN_DIR/routines.sock --agentd-socket $RUN_DIR/agentd.sock --routines-dir $STATE_DIR/routines
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

# mesh
cat > "$SYSTEMD_DIR/osmoda-mesh.service" <<EOF
[Unit]
Description=osModa Mesh P2P Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=$INSTALL_DIR/bin/osmoda-mesh --socket $RUN_DIR/mesh.sock --data-dir $STATE_DIR/mesh --agentd-socket $RUN_DIR/agentd.sock --listen-addr 0.0.0.0 --listen-port 18800$([ -n "$PUBLIC_IP" ] && echo " --public-addr ${PUBLIC_IP}:18800")
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

# Open mesh P2P port in firewall
if command -v ufw >/dev/null 2>&1; then
  ufw allow 18800/tcp 2>/dev/null || true
elif command -v nft >/dev/null 2>&1; then
  nft add rule inet filter input tcp dport 18800 accept 2>/dev/null || true
elif command -v iptables >/dev/null 2>&1; then
  iptables -A INPUT -p tcp --dport 18800 -j ACCEPT 2>/dev/null || true
fi

# mcpd
cat > "$SYSTEMD_DIR/osmoda-mcpd.service" <<EOF
[Unit]
Description=osModa MCP Server Manager
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=$INSTALL_DIR/bin/osmoda-mcpd --socket $RUN_DIR/mcpd.sock --state-dir $STATE_DIR/mcp --agentd-socket $RUN_DIR/agentd.sock
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

# teachd
cat > "$SYSTEMD_DIR/osmoda-teachd.service" <<EOF
[Unit]
Description=osModa Teaching/Learning Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=$INSTALL_DIR/bin/osmoda-teachd --socket $RUN_DIR/teachd.sock --state-dir $STATE_DIR/teachd --agentd-socket $RUN_DIR/agentd.sock --watch-socket $RUN_DIR/watch.sock
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

# voice
cat > "$SYSTEMD_DIR/osmoda-voice.service" <<EOF
[Unit]
Description=osModa Voice (STT/TTS)
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=$INSTALL_DIR/bin/osmoda-voice --socket $RUN_DIR/voice.sock --agentd-socket $RUN_DIR/agentd.sock
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

# egress
cat > "$SYSTEMD_DIR/osmoda-egress.service" <<EOF
[Unit]
Description=osModa Egress Proxy
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=$INSTALL_DIR/bin/osmoda-egress --port 3128 --state-dir $STATE_DIR
Restart=always
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=multi-user.target
EOF

# app-restore script (restores managed apps on boot from registry.json)
cat > "$INSTALL_DIR/bin/osmoda-app-restore.sh" <<'RESTOREOF'
#!/usr/bin/env bash
# osModa App Process Restore — runs on boot to recreate transient units from registry
# Skips apps marked as persistent (they have their own unit files)
set -euo pipefail

REGISTRY="/var/lib/osmoda/apps/registry.json"
if [ ! -f "$REGISTRY" ]; then
  echo "[app-restore] No registry found, nothing to restore"
  exit 0
fi

if ! command -v jq &>/dev/null; then
  echo "[app-restore] jq not found, cannot parse registry"
  exit 1
fi

APP_COUNT=0
FAIL_COUNT=0

jq -r '.apps | to_entries[] | select(.value.status == "running") | .key' "$REGISTRY" 2>/dev/null | while read -r APP_NAME; do
  # Skip apps with persistent unit files
  PERSISTENT=$(jq -r --arg n "$APP_NAME" '.apps[$n].persistent // false' "$REGISTRY")
  if [ "$PERSISTENT" = "true" ]; then
    echo "[app-restore] Skipping $APP_NAME (has persistent unit file)"
    continue
  fi

  COMMAND=$(jq -r --arg n "$APP_NAME" '.apps[$n].command' "$REGISTRY")
  if [ -z "$COMMAND" ] || [ "$COMMAND" = "null" ]; then
    echo "[app-restore] Skipping $APP_NAME (no command)"
    continue
  fi

  RESTART=$(jq -r --arg n "$APP_NAME" '.apps[$n].restart_policy // "on-failure"' "$REGISTRY")
  WORKDIR=$(jq -r --arg n "$APP_NAME" '.apps[$n].working_dir // empty' "$REGISTRY")
  MEMMAX=$(jq -r --arg n "$APP_NAME" '.apps[$n].memory_max // empty' "$REGISTRY")
  CPUQUOTA=$(jq -r --arg n "$APP_NAME" '.apps[$n].cpu_quota // empty' "$REGISTRY")
  USER=$(jq -r --arg n "$APP_NAME" '.apps[$n].user // empty' "$REGISTRY")

  SAFE_NAME=$(echo "$APP_NAME" | tr -cd 'a-zA-Z0-9_-')
  UNIT="osmoda-app-$SAFE_NAME"

  # Skip if unit already running
  if systemctl is-active "$UNIT" &>/dev/null; then
    echo "[app-restore] $APP_NAME ($UNIT) already running, skipping"
    continue
  fi

  # Build systemd-run command
  ARGS=()
  ARGS+=(--unit "$UNIT")
  ARGS+=(--service-type=simple)
  ARGS+=("--property=Restart=$RESTART")
  ARGS+=(--property=StartLimitIntervalSec=0)
  ARGS+=(--property=RestartSec=3)

  if [ -n "$USER" ] && [ "$USER" != "null" ]; then
    ARGS+=("--uid=$USER")
  fi

  [ -n "$WORKDIR" ] && [ "$WORKDIR" != "null" ] && ARGS+=("--working-directory=$WORKDIR")
  [ -n "$MEMMAX" ] && [ "$MEMMAX" != "null" ] && ARGS+=("--property=MemoryMax=$MEMMAX")
  [ -n "$CPUQUOTA" ] && [ "$CPUQUOTA" != "null" ] && ARGS+=("--property=CPUQuota=$CPUQUOTA")

  # Restore environment variables
  ENV_KEYS=$(jq -r --arg n "$APP_NAME" '.apps[$n].env // {} | keys[]' "$REGISTRY" 2>/dev/null)
  for KEY in $ENV_KEYS; do
    VAL=$(jq -r --arg n "$APP_NAME" --arg k "$KEY" '.apps[$n].env[$k]' "$REGISTRY")
    ARGS+=("--setenv=${KEY}=${VAL}")
  done

  # Command + args
  ARGS+=(--)
  ARGS+=("$COMMAND")
  ARG_COUNT=$(jq -r --arg n "$APP_NAME" '.apps[$n].args // [] | length' "$REGISTRY" 2>/dev/null)
  if [ "$ARG_COUNT" -gt 0 ] 2>/dev/null; then
    for i in $(seq 0 $((ARG_COUNT - 1))); do
      ARG=$(jq -r --arg n "$APP_NAME" --argjson i "$i" '.apps[$n].args[$i]' "$REGISTRY")
      ARGS+=("$ARG")
    done
  fi

  echo "[app-restore] Restoring: $APP_NAME → $UNIT"
  if systemd-run "${ARGS[@]}"; then
    APP_COUNT=$((APP_COUNT + 1))
  else
    FAIL_COUNT=$((FAIL_COUNT + 1))
    echo "[app-restore] FAIL: $APP_NAME"
  fi
done

echo "[app-restore] Done. Restored: $APP_COUNT, Failed: $FAIL_COUNT"
RESTOREOF
chmod +x "$INSTALL_DIR/bin/osmoda-app-restore.sh"

# app-restore systemd service (calls the script)
cat > "$SYSTEMD_DIR/osmoda-app-restore.service" <<'AREOF'
[Unit]
Description=osModa App Process Restore
After=network.target

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/opt/osmoda/bin/osmoda-app-restore.sh

[Install]
WantedBy=multi-user.target
AREOF

# WebSocket relay (bridges dashboard chat to local gateway)
if [ -n "$ORDER_ID" ] && [ -n "$CALLBACK_URL" ]; then

cat > "$INSTALL_DIR/bin/osmoda-ws-relay.js" <<'WSEOF'
#!/usr/bin/env node
// osModa WS Relay — bridges spawn.os.moda dashboard to local gateway.
// Supports both Claude Code SDK gateway and OpenClaw gateway (auto-detects).
const WebSocket = require("ws");
const fs = require("fs");
const crypto = require("crypto");

const STATE_DIR = "/var/lib/osmoda";
const RUNTIME = (() => { try { return fs.readFileSync(`${STATE_DIR}/config/runtime`, "utf8").trim(); } catch { return "openclaw"; } })();
const IDENTITY_DIR = RUNTIME === "claude-code" ? `${STATE_DIR}/identity` : "/root/.openclaw/identity";
const RECONNECT_DELAY = 5000;
const OC_URL = "ws://127.0.0.1:18789";

function readConfig(name) {
  try { return fs.readFileSync(`${STATE_DIR}/config/${name}`, "utf8").trim(); }
  catch { return ""; }
}
function uid() { return crypto.randomUUID(); }

function loadDeviceIdentity() {
  try {
    const device = JSON.parse(fs.readFileSync(`${IDENTITY_DIR}/device.json`, "utf8"));
    const auth = JSON.parse(fs.readFileSync(`${IDENTITY_DIR}/device-auth.json`, "utf8"));
    return { device, auth };
  } catch (e) {
    console.error("[ws-relay] no device identity found:", e.message);
    return null;
  }
}

function signPayload(privateKeyPem, payload) {
  return crypto.sign(null, Buffer.from(payload), privateKeyPem).toString("base64url");
}

function connect() {
  const orderId = readConfig("order-id");
  const callbackUrl = readConfig("callback-url");
  const secret = readConfig("heartbeat-secret");
  if (!orderId || !callbackUrl || !secret) {
    console.error("[ws-relay] missing config, retrying in 30s...");
    setTimeout(connect, 30000);
    return;
  }

  const identity = loadDeviceIdentity();
  if (!identity && RUNTIME === "openclaw") {
    console.error("[ws-relay] no device identity, retrying in 30s...");
    setTimeout(connect, 30000);
    return;
  }

  const wsBase = callbackUrl.replace(/^https?:\/\//, "").replace(/\/.*$/, "");
  const proto = callbackUrl.startsWith("https") ? "wss" : "ws";
  const ts = String(Date.now());
  const sig = crypto.createHmac("sha256", secret).update(`ws:${orderId}:${ts}`).digest("hex");

  const upstream = new WebSocket(`${proto}://${wsBase}/api/ws/agent/${orderId}`, {
    headers: { "x-ws-signature": sig, "x-ws-timestamp": ts },
  });

  let local = null;
  let ocReady = false;
  let connectId = null;
  let challengeNonce = null;
  let sessionKey = "spawn-" + orderId.slice(0, 8);
  let pendingChat = {};
  let instanceId = uid();

  // ─── Claude Code SDK: simple WS protocol ───
  function connectClaudeCode() {
    let ccLifecycleStarted = false;
    const gwToken = readConfig("gateway-token");
    local = new WebSocket(`${OC_URL}/ws`, {
      headers: { "Authorization": `Bearer ${gwToken}` },
    });
    local.on("open", () => {
      ocReady = true;
      console.log("[ws-relay] connected to Claude Code gateway");
      if (upstream.readyState === WebSocket.OPEN) {
        upstream.send(JSON.stringify({ type: "status", openclaw_connected: true, gateway_connected: true }));
      }
    });
    local.on("message", (data) => {
      let msg;
      try { msg = JSON.parse(data.toString()); } catch { return; }
      // Forward gateway events to upstream (spawn dashboard)
      // Convert from gateway format to spawn-expected format
      if (msg.type === "text") {
        // Send lifecycle start before first text (dashboard needs it to create stream container)
        if (!ccLifecycleStarted) { ccLifecycleStarted = true; upstream.send(JSON.stringify({ type: "event", event: "agent", payload: { stream: "lifecycle", data: { phase: "start" } } })); }
        upstream.send(JSON.stringify({ type: "event", event: "agent", payload: { stream: "assistant", data: { delta: msg.text } } }));
      } else if (msg.type === "tool_use") {
        if (!ccLifecycleStarted) { ccLifecycleStarted = true; upstream.send(JSON.stringify({ type: "event", event: "agent", payload: { stream: "lifecycle", data: { phase: "start" } } })); }
        upstream.send(JSON.stringify({ type: "event", event: "agent", payload: { stream: "tool_use", data: { name: msg.name, type: "tool_use" } } }));
      } else if (msg.type === "done") {
        if (ccLifecycleStarted) { upstream.send(JSON.stringify({ type: "event", event: "agent", payload: { stream: "lifecycle", data: { phase: "end" } } })); }
        ccLifecycleStarted = false;
        upstream.send(JSON.stringify({ type: "event", event: "agent", payload: { stream: "end" } }));
      } else if (msg.type === "error") {
        upstream.send(JSON.stringify({ type: "event", event: "error", payload: { message: msg.text } }));
      }
    });
    local.on("close", () => {
      console.log("[ws-relay] gateway disconnected, reconnecting...");
      ocReady = false; upstream.close();
    });
    local.on("error", (err) => console.error("[ws-relay] gateway error:", err.message));
  }

  // ─── OpenClaw: complex challenge/device identity protocol ───
  function sendOcConnect() {
    connectId = uid();
    const deviceToken = identity.auth.tokens?.operator?.token;
    const deviceId = identity.device.deviceId;
    const spkiB64 = identity.device.publicKeyPem.replace(/-----[^-]+-----/g, "").replace(/\s/g, "");
    const spkiBuf = Buffer.from(spkiB64, "base64");
    const publicKeyRaw = spkiBuf.subarray(spkiBuf.length - 32).toString("base64url");

    const params = {
      minProtocol: 3, maxProtocol: 3,
      client: { id: "gateway-client", version: "1.0.0", platform: "linux", mode: "cli", instanceId },
      role: "operator",
      scopes: ["operator.admin", "operator.write", "operator.read"],
      caps: [],
      userAgent: "osmoda-ws-relay/1.0", locale: "en",
      device: { id: deviceId, publicKey: publicKeyRaw },
      auth: { deviceToken, token: readConfig("gateway-token") }
    };

    if (challengeNonce) {
      const now = Date.now();
      const authToken = readConfig("gateway-token") || deviceToken || "";
      const scopeStr = params.scopes.join(",");
      const payload = `v3|${deviceId}|${params.client.id}|${params.client.mode}|${params.role}|${scopeStr}|${now}|${authToken}|${challengeNonce}|${params.client.platform}|`;
      params.device.signature = signPayload(identity.device.privateKeyPem, payload);
      params.device.nonce = challengeNonce;
      params.device.signedAt = now;
    }

    local.send(JSON.stringify({ type: "req", id: connectId, method: "connect", params }));
  }

  function connectOpenClaw() {
    local = new WebSocket(OC_URL, { headers: { origin: "http://127.0.0.1:18789" } });

    local.on("open", () => {
      console.log("[ws-relay] connected to OpenClaw, waiting for challenge...");
    });

    local.on("message", (data) => {
      let msg;
      try { msg = JSON.parse(data.toString()); } catch { return; }

      if (!ocReady) {
        if (msg.type === "event" && msg.event === "connect.challenge") {
          challengeNonce = msg.payload?.nonce || msg.nonce;
          console.log("[ws-relay] got challenge, sending connect with device identity...");
          sendOcConnect();
          return;
        }
        if (msg.type === "res" && msg.id === connectId) {
          if (msg.ok) {
            ocReady = true;
            const scopes = msg.payload?.auth?.scopes || [];
            console.log("[ws-relay] handshake complete, scopes:", JSON.stringify(scopes));
            if (upstream.readyState === WebSocket.OPEN) {
              upstream.send(JSON.stringify({ type: "status", openclaw_connected: true }));
            }
          } else {
            console.error("[ws-relay] connect rejected:", JSON.stringify(msg.error));
            local.close();
          }
          return;
        }
        return;
      }

      if (msg.type === "event" || msg.type === "res") {
        if (upstream.readyState === WebSocket.OPEN) upstream.send(JSON.stringify(msg));
        if (msg.type === "res") delete pendingChat[msg.id];
      }
    });

    local.on("close", () => {
      console.log("[ws-relay] OpenClaw disconnected, reconnecting...");
      ocReady = false; challengeNonce = null; upstream.close();
    });
    local.on("error", (err) => console.error("[ws-relay] OpenClaw error:", err.message));
  }

  upstream.on("open", () => {
    console.log("[ws-relay] connected to spawn server");
    if (RUNTIME === "claude-code") {
      connectClaudeCode();
    } else {
      connectOpenClaw();
    }
  });

  upstream.on("message", (data) => {
    if (!local || local.readyState !== WebSocket.OPEN || !ocReady) return;
    let msg;
    try { msg = JSON.parse(data.toString()); } catch { return; }

    if (msg.type === "chat" && msg.text) {
      if (RUNTIME === "claude-code") {
        // Claude Code gateway: simple chat message
        local.send(JSON.stringify({ type: "chat", text: msg.text, sessionKey: sessionKey }));
      } else {
        // OpenClaw: chat.send RPC
        const reqId = uid();
        pendingChat[reqId] = true;
        local.send(JSON.stringify({
          type: "req", id: reqId, method: "chat.send",
          params: { message: msg.text, idempotencyKey: reqId, sessionKey: sessionKey }
        }));
      }
      console.log("[ws-relay] chat:", msg.text.slice(0, 50));
      return;
    }
    if (msg.type === "chat_abort") {
      if (RUNTIME === "claude-code") {
        local.send(JSON.stringify({ type: "abort" }));
      } else {
        local.send(JSON.stringify({ type: "req", id: uid(), method: "chat.abort", params: { sessionKey } }));
      }
    }
  });

  upstream.on("close", () => {
    console.log("[ws-relay] spawn disconnected, reconnecting...");
    ocReady = false; challengeNonce = null;
    if (local) local.close();
    setTimeout(connect, RECONNECT_DELAY);
  });
  upstream.on("error", (err) => console.error("[ws-relay] spawn error:", err.message));
}

connect();
WSEOF
chmod +x "$INSTALL_DIR/bin/osmoda-ws-relay.js"

cat > "$SYSTEMD_DIR/osmoda-ws-relay.service" <<EOF
[Unit]
Description=osModa WebSocket Chat Relay
After=osmoda-gateway.service
Wants=osmoda-gateway.service
[Service]
Type=simple
ExecStart=/bin/sh -c 'exec node $INSTALL_DIR/bin/osmoda-ws-relay.js'
Restart=always
RestartSec=5
Environment=NODE_PATH=$INSTALL_DIR/packages/osmoda-gateway/node_modules:$OPENCLAW_DIR/node_modules
[Install]
WantedBy=multi-user.target
EOF

fi # end ORDER_ID check for ws-relay

# Heartbeat timer (phones home to spawn.os.moda)
if [ -n "$ORDER_ID" ] && [ -n "$CALLBACK_URL" ]; then

# Ensure jq is available for heartbeat action processing
if ! command -v jq &>/dev/null; then
  if command -v nix-env &>/dev/null; then
    nix-env -iA nixos.jq 2>/dev/null || true
  elif command -v apt-get &>/dev/null; then
    apt-get install -y -qq jq 2>/dev/null || true
  fi
fi

# Write heartbeat script (more capable than inline bash)
cat > "$INSTALL_DIR/bin/osmoda-heartbeat.sh" <<'HBEOF'
#!/usr/bin/env bash
set -o pipefail
STATE_DIR="/var/lib/osmoda"
RUN_DIR="/run/osmoda"

# Ensure only one heartbeat runs at a time (prevent race conditions)
exec 200>/var/run/osmoda-heartbeat.lock
flock -n 200 || exit 0

# jq is required for JSON processing — check early
if ! command -v jq &>/dev/null; then echo "jq not found — heartbeat disabled"; exit 0; fi

OID=$(cat "$STATE_DIR/config/order-id" 2>/dev/null) || exit 0
CBURL=$(cat "$STATE_DIR/config/callback-url" 2>/dev/null) || exit 0
HBSECRET=$(cat "$STATE_DIR/config/heartbeat-secret" 2>/dev/null || echo "")
INSTALL_DIR="/opt/osmoda"

# Self-heal mesh daemon config (one-time migration for servers deployed before --public-addr)
MESH_SERVICE_FILE="/etc/systemd/system/osmoda-mesh.service"
[ ! -w "$MESH_SERVICE_FILE" ] && MESH_SERVICE_FILE="/run/systemd/system/osmoda-mesh.service"
if [ -f "$MESH_SERVICE_FILE" ] && ! grep -q "listen-addr 0.0.0.0" "$MESH_SERVICE_FILE"; then
  MESH_PUBLIC_IP=$(curl -sf -m 3 http://169.254.169.254/hetzner/v1/metadata/public-ipv4 2>/dev/null || curl -sf -m 3 https://ifconfig.me 2>/dev/null || hostname -I | awk '{print $1}')
  # Validate IP format before using in sed
  if [ -n "$MESH_PUBLIC_IP" ] && echo "$MESH_PUBLIC_IP" | grep -qE '^[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}$'; then
    sed -i "s|--listen-port 18800|--listen-addr 0.0.0.0 --listen-port 18800 --public-addr ${MESH_PUBLIC_IP}:18800|" "$MESH_SERVICE_FILE"
    systemctl daemon-reload
    systemctl restart osmoda-mesh 2>/dev/null || true
    # Open firewall port for mesh P2P
    if command -v ufw >/dev/null 2>&1; then ufw allow 18800/tcp 2>/dev/null || true
    elif command -v iptables >/dev/null 2>&1; then iptables -A INPUT -p tcp --dport 18800 -j ACCEPT 2>/dev/null || true; fi
  fi
fi

# Self-heal Telegram security: close open access on existing servers (one-time migration) — openclaw only
RUNTIME_CFG=$(cat "$STATE_DIR/config/runtime" 2>/dev/null || echo "openclaw")
OPENCLAW_CONFIG="/root/.openclaw/openclaw.json"
GATEWAY_CONFIG="$STATE_DIR/config/gateway.json"
if [ "$RUNTIME_CFG" = "openclaw" ] && [ -f "$OPENCLAW_CONFIG" ] && command -v node >/dev/null 2>&1; then
  # Check returns exit 1 ONLY if wildcard found; exit 0 for clean or errors
  if node - <<'TGCHKEOF'
try{var c=JSON.parse(require('fs').readFileSync('/root/.openclaw/openclaw.json','utf8'));
var ch=c.channels||{};var bad=false;Object.keys(ch).forEach(function(k){if(ch[k].allowFrom&&ch[k].allowFrom[0]==='*')bad=true;});
process.exit(bad?1:0);}catch(e){process.exit(0);}
TGCHKEOF
  then
    : # no wildcard found or parse error, skip fix
  else
    node - <<'TGSECEOF'
var fs=require('fs');
var config=JSON.parse(fs.readFileSync('/root/.openclaw/openclaw.json','utf8'));
var ch=config.channels||{};
var fixed=false;
Object.keys(ch).forEach(function(k){
  if(ch[k].allowFrom&&ch[k].allowFrom[0]==='*'){
    ch[k].allowFrom=[];
    ch[k].dmPolicy='pairing';
    fixed=true;
  }
});
if(fixed){
  fs.writeFileSync('/root/.openclaw/openclaw.json',JSON.stringify(config,null,2));
  console.log('[self-heal] Closed wildcard Telegram access — switched to pairing mode');
}
TGSECEOF
    systemctl restart osmoda-gateway.service 2>/dev/null || true
  fi
fi

# Self-heal: fix Hetzner password expiry blocking SSH key auth
# Hetzner may re-set password expiry after cloud-init; this ensures SSH always works
if chage -l root 2>/dev/null | grep -q "password must be changed"; then
  passwd -d root 2>/dev/null || true
  chage -I -1 -m 0 -M 99999 -E -1 root 2>/dev/null || true
fi

# Collect health from agentd (5s timeout to prevent hangs)
HEALTH=$(curl -sf --max-time 5 --unix-socket "$RUN_DIR/agentd.sock" http://l/health 2>/dev/null || echo "{}")
# Parse with jq — agentd returns cpu_usage[] (per-core), memory_total/used (bytes), disks[], uptime (secs)
# cpu_usage[] from agentd is already 0-100 percentage per core (sysinfo cpu_usage())
CPU=$(echo "$HEALTH" | jq '[.cpu_usage[]? // 0] | if length > 0 then [((add / length) | round), 100] | min else 0 end' 2>/dev/null || echo 0)
MEM_TOTAL=$(echo "$HEALTH" | jq '.memory_total // 0' 2>/dev/null || echo 0)
MEM_USED=$(echo "$HEALTH" | jq '.memory_used // 0' 2>/dev/null || echo 0)
RAM=$(echo "$MEM_TOTAL $MEM_USED" | awk '{if ($1 > 0) printf "%.1f", $2/$1*100; else print 0}')
DISK=$(echo "$HEALTH" | jq '(.disks[0] // {}) | if .total > 0 then (.used / .total * 100) else 0 end' 2>/dev/null || echo 0)
UPTIME=$(echo "$HEALTH" | jq '.uptime // 0' 2>/dev/null || echo 0)
OC_READY=$(systemctl is-active osmoda-gateway.service 2>/dev/null | grep -q "^active$" && echo true || echo false)

# Build completed_actions from previous heartbeat
COMPLETED_FILE="$STATE_DIR/config/completed-actions"
COMPLETED_JSON="[]"
if [ -f "$COMPLETED_FILE" ] && [ -s "$COMPLETED_FILE" ]; then
  COMPLETED_JSON=$(cat "$COMPLETED_FILE")
fi

# Collect agent instances (from gateway.json or openclaw.json)
AGENTS_JSON="[]"
GW_ACTIVE="false"
systemctl is-active osmoda-gateway.service >/dev/null 2>&1 && GW_ACTIVE="true"

if [ "$RUNTIME_CFG" = "claude-code" ] && [ -f "$GATEWAY_CONFIG" ]; then
  # Claude Code: parse gateway.json for agent info
  AGENTS_JSON=$(jq -c --arg status "$([ "$GW_ACTIVE" = "true" ] && echo running || echo stopped)" \
    '[.agents[]? | {name: .id, status: $status, model: .model, channels: [], "default": (.default // false)}]' \
    "$GATEWAY_CONFIG" 2>/dev/null || echo "[]")
  # Enrich with binding channels
  for agent_id in $(jq -r '.agents[]?.id' "$GATEWAY_CONFIG" 2>/dev/null); do
    ACHANNELS=$(jq -c --arg id "$agent_id" '[.bindings[]? | select(.agentId == $id) | .channel]' "$GATEWAY_CONFIG" 2>/dev/null || echo "[]")
    AGENTS_JSON=$(echo "$AGENTS_JSON" | jq --arg id "$agent_id" --argjson ch "$ACHANNELS" \
      '[.[] | if .name == $id then .channels = $ch else . end]')
  done
elif [ -d /root/.openclaw/agents ]; then
  # OpenClaw: parse openclaw.json + agent dirs
  OC_CONFIG=""
  [ -f /root/.openclaw/openclaw.json ] && OC_CONFIG=$(cat /root/.openclaw/openclaw.json 2>/dev/null)

  for agent_dir in /root/.openclaw/agents/*/; do
    [ -d "$agent_dir" ] || continue
    ANAME=$(basename "$agent_dir")
    ASTATUS="stopped"
    [ "$GW_ACTIVE" = "true" ] && ASTATUS="running"

    AMODEL=""
    ACHANNELS="[]"
    ADEFAULT="false"
    if [ -n "$OC_CONFIG" ]; then
      AMODEL=$(echo "$OC_CONFIG" | jq -r --arg id "$ANAME" '.agents.list[]? | select(.id == $id) | .model // ""' 2>/dev/null)
      ADEFAULT=$(echo "$OC_CONFIG" | jq -r --arg id "$ANAME" '.agents.list[]? | select(.id == $id) | .default // false' 2>/dev/null)
      ACHANNELS=$(echo "$OC_CONFIG" | jq -c --arg id "$ANAME" '[.bindings[]? | select(.agentId == $id) | .match.channel // empty]' 2>/dev/null || echo "[]")
    fi

    AGENTS_JSON=$(echo "$AGENTS_JSON" | jq \
      --arg name "$ANAME" \
      --arg status "$ASTATUS" \
      --arg model "${AMODEL:-}" \
      --argjson channels "${ACHANNELS:-[]}" \
      --argjson isDefault "${ADEFAULT:-false}" \
      '. + [{name: $name, status: $status, model: $model, channels: $channels, "default": $isDefault}]')
  done
fi

# Collect daemon health
DAEMON_JSON="{}"
for svc in agentd keyd watch routines mesh mcpd teachd voice egress gateway; do
  UNIT="osmoda-${svc}.service"
  DACTIVE=$(systemctl is-active "$UNIT" 2>/dev/null || echo "inactive")
  DPID=$(systemctl show -p MainPID --value "$UNIT" 2>/dev/null || echo "0")
  DAEMON_JSON=$(echo "$DAEMON_JSON" | jq \
    --arg name "$svc" \
    --argjson active "$([ "$DACTIVE" = "active" ] && echo true || echo false)" \
    --argjson pid "${DPID:-0}" \
    '.[$name] = {active: $active, pid: (if $pid == 0 then null else $pid end)}')
done

# Collect mesh identity + peers (5s timeout)
MESH_IDENTITY=$(curl -sf --max-time 5 --unix-socket "$RUN_DIR/mesh.sock" http://l/identity 2>/dev/null || echo "{}")
MESH_PEERS=$(curl -sf --max-time 5 --unix-socket "$RUN_DIR/mesh.sock" http://l/peers 2>/dev/null || echo "[]")
MESH_PEERS_SLIM=$(echo "$MESH_PEERS" | jq '[.[] | {id: .id, label: .label, state: (.connection_state.state // .connection_state // "unknown")}]' 2>/dev/null || echo "[]")

# Collect routines from routines daemon (3s timeout)
ROUTINES="[]"
ROUTINE_HISTORY="[]"
if systemctl is-active osmoda-routines &>/dev/null; then
  ROUTINES=$(curl -sf --max-time 3 --unix-socket "$RUN_DIR/routines.sock" http://l/routine/list 2>/dev/null || echo "[]")
  ROUTINE_HISTORY=$(curl -sf --max-time 3 --unix-socket "$RUN_DIR/routines.sock" http://l/routine/history 2>/dev/null || echo "[]")
fi

# Collect watchers + SafeSwitch sessions from watch daemon (3s timeout)
WATCHERS="[]"
SWITCH_SESSIONS="[]"
if systemctl is-active osmoda-watch &>/dev/null; then
  WATCHERS=$(curl -sf --max-time 3 --unix-socket "$RUN_DIR/watch.sock" http://l/watcher/list 2>/dev/null || echo "[]")
  SWITCH_SESSIONS=$(curl -sf --max-time 3 --unix-socket "$RUN_DIR/watch.sock" http://l/switch/list 2>/dev/null || echo "[]")
fi

# Collect NixOS generation info
NIXOS_GENERATION=""
if [ -L /nix/var/nix/profiles/system ]; then
  NIXOS_GENERATION=$(readlink /nix/var/nix/profiles/system 2>/dev/null || echo "")
fi

# Collect recent events from agentd (3s timeout)
RECENT_EVENTS="[]"
if systemctl is-active osmoda-agentd &>/dev/null; then
  RECENT_EVENTS=$(curl -sf --max-time 3 --unix-socket "$RUN_DIR/agentd.sock" "http://l/events/log?limit=30" 2>/dev/null || echo "[]")
fi

# Collect teachd health + patterns (3s timeout)
TEACHD_HEALTH="{}"
TEACHD_PATTERNS="[]"
if systemctl is-active osmoda-teachd &>/dev/null; then
  TEACHD_HEALTH=$(curl -sf --max-time 3 --unix-socket "$RUN_DIR/teachd.sock" http://l/health 2>/dev/null || echo "{}")
  TEACHD_PATTERNS=$(curl -sf --max-time 3 --unix-socket "$RUN_DIR/teachd.sock" "http://l/patterns?min_confidence=0.7&limit=10" 2>/dev/null || echo "[]")
fi

# Collect MCP servers from mcpd (3s timeout)
MCP_SERVERS="[]"
if systemctl is-active osmoda-mcpd &>/dev/null; then
  MCP_SERVERS=$(curl -sf --max-time 3 --unix-socket "$RUN_DIR/mcpd.sock" http://l/servers 2>/dev/null || echo "[]")
fi

# Detect running apps: osmoda-app-* services + /home project dirs + listening ports
APPS_JSON=$(python3 -c '
import subprocess, os, json, re
apps = []
# 1. osmoda-app-* systemd services
try:
    out = subprocess.check_output(["systemctl","list-units","--type=service","--state=running","--no-legend","--plain"], text=True, timeout=5)
    for line in out.strip().split("\n"):
        if not line.strip(): continue
        unit = line.split()[0]
        if unit.startswith("osmoda-app-"):
            app_name = unit.replace("osmoda-app-","").replace(".service","")
            try:
                d = subprocess.check_output(["systemctl","show",unit,"--property=ExecStart","--no-pager"], text=True, timeout=3)
                cmd = d.strip().split("=",1)[-1][:100] if "=" in d else ""
            except: cmd = ""
            apps.append({"type":"app","name":app_name,"status":"running","command":cmd,"unit":unit})
except: pass
# 2. Project dirs in /home
try:
    if os.path.isdir("/home"):
        for d in os.listdir("/home"):
            hp = os.path.join("/home", d)
            if not os.path.isdir(hp): continue
            for mf, lang in [("requirements.txt","python"),("package.json","node"),("Cargo.toml","rust"),("go.mod","go"),("main.py","python"),("app.py","python"),("index.js","node"),("index.ts","node")]:
                if os.path.isfile(os.path.join(hp, mf)):
                    apps.append({"type":"project","name":d,"path":"/home/"+d,"language":lang,"marker":mf})
                    break
except: pass
# 3. Listening ports (skip system services)
try:
    out = subprocess.check_output(["ss","-tlnp"], text=True, timeout=5)
    for line in out.strip().split("\n")[1:]:
        parts = line.split()
        if len(parts) < 5: continue
        local = parts[3]
        port = local.rsplit(":",1)[-1] if ":" in local else ""
        proc = parts[5] if len(parts) > 5 else ""
        m = re.search(r"users:\(\(\"([^\"]+)\"", proc)
        name = m.group(1) if m else ""
        if name in ("sshd","systemd-resolve","osmoda-egress","node","agentd","osmoda-keyd","osmoda-watch","osmoda-routines","osmoda-mesh","osmoda-mcpd","osmoda-teachd","osmoda-voice","nginx","caddy"): continue
        if port and port.isdigit() and int(port) > 1024:
            apps.append({"type":"port","name":name or "unknown","port":int(port)})
except: pass
# 4. Docker/podman containers
for rt in ["docker","podman"]:
    try:
        out = subprocess.check_output([rt,"ps","--format","{{.Names}}\\t{{.Status}}\\t{{.Ports}}\\t{{.Image}}"], text=True, timeout=5)
        for line in out.strip().split("\n"):
            if not line.strip(): continue
            p = line.split("\\t")
            apps.append({"type":"container","name":p[0],"status":p[1] if len(p)>1 else "","ports":p[2] if len(p)>2 else "","image":p[3] if len(p)>3 else "","runtime":rt})
    except: pass
# 5. Nginx vhosts
try:
    import glob as g
    for conf in g.glob("/etc/nginx/sites-enabled/*") + g.glob("/etc/nginx/conf.d/*.conf") + ["/etc/nginx/nginx.conf"]:
        if os.path.isfile(conf):
            content = open(conf).read()
            for m in re.finditer(r"server_name\s+([^;]+);", content):
                for n in m.group(1).split():
                    if n not in ("_","localhost","") and not re.match(r"^\d+\.\d+\.\d+\.\d+$", n):
                        apps.append({"type":"domain","name":n,"source":"nginx","conf":os.path.basename(conf)})
except: pass
# 6. Caddy domains
try:
    import glob as g
    for cf in g.glob("/etc/caddy/Caddyfile") + g.glob("/etc/caddy/conf.d/*"):
        if os.path.isfile(cf):
            for m in re.finditer(r"^([a-zA-Z0-9][\w.-]+\.\w{2,})\s*\{", open(cf).read(), re.MULTILINE):
                apps.append({"type":"domain","name":m.group(1),"source":"caddy","conf":os.path.basename(cf)})
except: pass
# 7. Project dirs in /root (user apps)
try:
    for d in os.listdir("/root"):
        dp = os.path.join("/root", d)
        if not os.path.isdir(dp) or d.startswith("."): continue
        if d in ("workspace",): continue
        for mf, lang in [("package.json","node"),("requirements.txt","python"),("Cargo.toml","rust"),("go.mod","go"),("main.py","python"),("app.py","python"),("index.js","node"),("index.ts","node")]:
            if os.path.isfile(os.path.join(dp, mf)):
                apps.append({"type":"project","name":d,"path":"/root/"+d,"language":lang,"marker":mf})
                break
except: pass
# 8. osModa managed apps (/var/lib/osmoda/apps)
try:
    app_dir = "/var/lib/osmoda/apps"
    if os.path.isdir(app_dir):
        for d in os.listdir(app_dir):
            mf = os.path.join(app_dir, d, "app.json")
            if os.path.isfile(mf):
                m = json.loads(open(mf).read())
                apps.append({"type":"managed","name":m.get("name",d),"domain":m.get("domain",""),"status":m.get("status","unknown"),"port":m.get("port",0)})
except: pass
# Deduplicate
seen = set()
unique = []
for a in apps:
    key = a["type"] + ":" + a.get("name","") + ":" + str(a.get("port",""))
    if key not in seen:
        seen.add(key)
        unique.append(a)
print(json.dumps(unique))
' 2>/dev/null || echo "[]")

# Build payload (use jq for safe JSON construction)
PAYLOAD=$(jq -n \
  --arg oid "$OID" \
  --argjson oc_ready "$OC_READY" \
  --argjson cpu "${CPU:-0}" \
  --argjson ram "${RAM:-0}" \
  --argjson disk "${DISK:-0}" \
  --argjson uptime "${UPTIME:-0}" \
  --argjson completed "$COMPLETED_JSON" \
  --argjson agents "$AGENTS_JSON" \
  --argjson daemon_health "$DAEMON_JSON" \
  --argjson mesh_identity "$MESH_IDENTITY" \
  --argjson mesh_peers "$MESH_PEERS_SLIM" \
  --argjson routines "$ROUTINES" \
  --argjson routine_history "$ROUTINE_HISTORY" \
  --argjson watchers "$WATCHERS" \
  --argjson switch_sessions "$SWITCH_SESSIONS" \
  --arg nixos_generation "$NIXOS_GENERATION" \
  --argjson recent_events "$RECENT_EVENTS" \
  --argjson teachd_health "$TEACHD_HEALTH" \
  --argjson teachd_patterns "$TEACHD_PATTERNS" \
  --argjson mcp_servers "$MCP_SERVERS" \
  --argjson apps "$APPS_JSON" \
  --arg runtime "$RUNTIME_CFG" \
  '{order_id: $oid, status: "alive", setup_complete: true, openclaw_ready: $oc_ready, gateway_ready: $oc_ready, runtime: $runtime, health: {cpu: $cpu, ram: $ram, disk: $disk, uptime: $uptime}, completed_actions: $completed, agents: $agents, daemon_health: $daemon_health, mesh_identity: $mesh_identity, mesh_peers: $mesh_peers, routines: $routines, routine_history: $routine_history, watchers: $watchers, switch_sessions: $switch_sessions, nixos_generation: $nixos_generation, recent_events: $recent_events, teachd_health: $teachd_health, teachd_patterns: $teachd_patterns, mcp_servers: $mcp_servers, apps: $apps}'
)

# Send heartbeat (with HMAC signature if secret is set)
if [ -n "$HBSECRET" ]; then
  HB_TS=$(date +%s000)
  SIGNATURE=$(printf '%s:%s' "$OID" "$HB_TS" | openssl dgst -sha256 -hmac "$HBSECRET" 2>/dev/null | awk '{print $NF}')
  RESPONSE=$(curl -sf --max-time 15 -X POST "$CBURL" \
    -H "Content-Type: application/json" \
    -H "X-Heartbeat-Signature: $SIGNATURE" \
    -H "X-Heartbeat-Timestamp: $HB_TS" \
    -d "$PAYLOAD" 2>/dev/null) || exit 0
else
  RESPONSE=$(curl -sf --max-time 15 -X POST "$CBURL" \
    -H "Content-Type: application/json" \
    -d "$PAYLOAD" 2>/dev/null) || exit 0
fi

# Process pending actions from response (completed file is overwritten at end with new completions)

NEW_COMPLETED="[]"
for ACTION_B64 in $(echo "$RESPONSE" | jq -r '.actions[]? | @base64' 2>/dev/null); do
  ACTION_JSON=$(echo "$ACTION_B64" | base64 -d 2>/dev/null) || continue
  ATYPE=$(echo "$ACTION_JSON" | jq -r '.type' 2>/dev/null) || continue
  AID=$(echo "$ACTION_JSON" | jq -r '.id' 2>/dev/null) || continue

  case "$ATYPE" in
    add_ssh_key)
      AKEY=$(echo "$ACTION_JSON" | jq -r '.key' 2>/dev/null) || continue
      if [ -n "$AKEY" ] && [ "$AKEY" != "null" ]; then
        mkdir -p /root/.ssh && chmod 700 /root/.ssh
        if ! grep -qF "$AKEY" /root/.ssh/authorized_keys 2>/dev/null; then
          printf '%s\n' "$AKEY" >> /root/.ssh/authorized_keys
          chmod 600 /root/.ssh/authorized_keys
        fi
        NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" '. + [$id]')
      fi
      ;;
    remove_ssh_key)
      AFP=$(echo "$ACTION_JSON" | jq -r '.fingerprint' 2>/dev/null) || continue
      if [ -n "$AFP" ] && [ "$AFP" != "null" ] && [ -f /root/.ssh/authorized_keys ]; then
        AK_TMP=$(mktemp /root/.ssh/.ak_tmp.XXXXXX) || continue
        grep -vF "$AFP" /root/.ssh/authorized_keys > "$AK_TMP" 2>/dev/null && mv "$AK_TMP" /root/.ssh/authorized_keys || rm -f "$AK_TMP"
        chmod 600 /root/.ssh/authorized_keys
      fi
      NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" '. + [$id]')
      ;;
    mesh_invite_create)
      # Create a mesh invite on this server, report back the invite code
      TARGET_IP=$(echo "$ACTION_JSON" | jq -r '.target_server_ip' 2>/dev/null)
      TARGET_PORT=$(echo "$ACTION_JSON" | jq -r '.target_mesh_port' 2>/dev/null)
      # Detect own public IP so the invite code contains a routable endpoint
      MY_PUBLIC_IP=$(curl -sf -m 3 http://169.254.169.254/hetzner/v1/metadata/public-ipv4 2>/dev/null || curl -sf -m 3 https://ifconfig.me 2>/dev/null || hostname -I | awk '{print $1}')
      # Validate IP format
      if ! echo "$MY_PUBLIC_IP" | grep -qE '^[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}$'; then
        echo "mesh_invite_create: failed to detect valid public IP"
        NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" '. + [$id]')
        continue
      fi
      INVITE_BODY=$(jq -n --argjson ttl 3600 --arg ep "${MY_PUBLIC_IP}:18800" '{ttl_secs: $ttl, endpoint: $ep}')
      INVITE_RESULT=$(curl -sf --max-time 10 --unix-socket "$RUN_DIR/mesh.sock" \
        -X POST http://l/invite/create \
        -H "Content-Type: application/json" \
        -d "$INVITE_BODY" 2>/dev/null)
      INVITE_CODE=$(echo "$INVITE_RESULT" | jq -r '.invite_code' 2>/dev/null)
      if [ -n "$INVITE_CODE" ] && [ "$INVITE_CODE" != "null" ]; then
        # Report as completed with invite_code result (for relay to target server)
        NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" --arg code "$INVITE_CODE" \
          '. + [{id: $id, result: {invite_code: $code}}]')
      else
        NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" '. + [$id]')
      fi
      ;;
    mesh_invite_accept)
      # Accept a mesh invite on this server
      INVITE_CODE=$(echo "$ACTION_JSON" | jq -r '.invite_code' 2>/dev/null)
      if [ -n "$INVITE_CODE" ] && [ "$INVITE_CODE" != "null" ]; then
        # Use jq for safe JSON construction (no shell interpolation of invite code)
        ACCEPT_BODY=$(jq -n --arg code "$INVITE_CODE" '{invite_code: $code}')
        ACCEPT_RESULT=$(curl -sf --max-time 15 --unix-socket "$RUN_DIR/mesh.sock" \
          -X POST http://l/invite/accept \
          -H "Content-Type: application/json" \
          -d "$ACCEPT_BODY" 2>&1)
        ACCEPT_EXIT=$?
        if [ $ACCEPT_EXIT -eq 0 ] && [ -n "$ACCEPT_RESULT" ]; then
          PEER_ID=$(echo "$ACCEPT_RESULT" | jq -r '.peer_id // empty' 2>/dev/null)
          NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" --arg pid "${PEER_ID:-unknown}" \
            '. + [{id: $id, result: {peer_id: $pid, status: "connected"}}]')
        else
          ACCEPT_ERR=$(echo "$ACCEPT_RESULT" | head -c 200)
          NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" --arg err "${ACCEPT_ERR:-mesh daemon unreachable}" \
            '. + [{id: $id, result: {status: "failed", error: $err}}]')
        fi
      fi
      ;;
    mesh_peer_disconnect)
      # Disconnect a mesh peer
      PEER_ID=$(echo "$ACTION_JSON" | jq -r '.peer_instance_id' 2>/dev/null)
      # Sanitize peer ID: must be hex/alphanumeric, no slashes or special chars
      PEER_ID=$(echo "$PEER_ID" | tr -cd 'a-zA-Z0-9_-')
      if [ -n "$PEER_ID" ] && [ "$PEER_ID" != "null" ]; then
        curl -sf --max-time 10 --unix-socket "$RUN_DIR/mesh.sock" \
          -X DELETE "http://l/peer/$PEER_ID" 2>/dev/null || true
        NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" '. + [$id]')
      fi
      ;;
    update_api_key)
      APROVIDER=$(echo "$ACTION_JSON" | jq -r '.provider' 2>/dev/null)
      AKEY=$(echo "$ACTION_JSON" | jq -r '.key' 2>/dev/null) || continue
      if [ -n "$AKEY" ] && [ "$AKEY" != "null" ]; then
        # Decrypt API key if encrypted (ENC:iv:tag:ciphertext format from spawn server)
        if echo "$AKEY" | grep -q '^ENC:' && [ -n "$HBSECRET" ]; then
          ENC_IV=$(echo "$AKEY" | cut -d: -f2)
          ENC_TAG=$(echo "$AKEY" | cut -d: -f3)
          ENC_CT=$(echo "$AKEY" | cut -d: -f4)
          AKEY=$(echo "$ENC_CT" | openssl enc -aes-256-gcm -d -K "$(echo -n "$HBSECRET" | openssl dgst -sha256 -binary | xxd -p -c 64)" -iv "$ENC_IV" -nopad 2>/dev/null | xxd -r -p 2>/dev/null) || true
          # openssl enc doesn't support GCM well in all versions — fall back to node
          if [ -z "$AKEY" ] || echo "$AKEY" | grep -q '^ENC:'; then
            AKEY=$(node - "$HBSECRET" "$ENC_IV" "$ENC_TAG" "$ENC_CT" <<'DECEOF'
var c=require('crypto'),s=process.argv[2],iv=process.argv[3],tag=process.argv[4],ct=process.argv[5];
var dk=c.createHash('sha256').update(s).digest();
var d=c.createDecipheriv('aes-256-gcm',dk,Buffer.from(iv,'hex'));
d.setAuthTag(Buffer.from(tag,'hex'));
var pt=d.update(ct,'hex','utf8')+d.final('utf8');
process.stdout.write(pt);
DECEOF
            ) || true
          fi
          if [ -z "$AKEY" ]; then
            echo "Failed to decrypt API key — skipping action $AID"
            continue
          fi
        fi
        # Update stored API key
        printf '%s\n' "$AKEY" > "$STATE_DIR/config/api-key"
        chmod 600 "$STATE_DIR/config/api-key"
        # Update env file (detect OAuth tokens vs Console API keys)
        if [ "$APROVIDER" = "openai" ]; then
          printf 'OPENAI_API_KEY=%s\n' "$AKEY" > "$STATE_DIR/config/env"
        elif echo "$AKEY" | grep -q '^sk-ant-oat'; then
          printf 'CLAUDE_CODE_OAUTH_TOKEN=%s\n' "$AKEY" > "$STATE_DIR/config/env"
        else
          printf 'ANTHROPIC_API_KEY=%s\n' "$AKEY" > "$STATE_DIR/config/env"
        fi
        chmod 600 "$STATE_DIR/config/env"
        # Update auth-profiles.json for all agents (openclaw only)
        if [ "$RUNTIME_CFG" = "openclaw" ]; then
          SAFE_PROVIDER="anthropic"
          if [ "$APROVIDER" = "openai" ]; then SAFE_PROVIDER="openai"; fi
          AUTH_TYPE="api_key"
          AUTH_FIELD="key"
          if [ "$SAFE_PROVIDER" = "anthropic" ] && echo "$AKEY" | grep -q '^sk-ant-oat'; then
            AUTH_TYPE="token"
            AUTH_FIELD="token"
          fi
          for _AGID in osmoda mobile; do
            mkdir -p "/root/.openclaw/agents/$_AGID/agent"
            jq -n --arg type "$AUTH_TYPE" --arg provider "$SAFE_PROVIDER" --arg key "$AKEY" --arg field "$AUTH_FIELD" \
              '{type: $type, provider: $provider, ($field): $key}' \
              > "/root/.openclaw/agents/$_AGID/agent/auth-profiles.json"
          done
        fi
        # Ensure gateway token exists for relay auth
        HBGWTOKEN=""
        if [ -f "$STATE_DIR/config/gateway-token" ]; then
          HBGWTOKEN=$(cat "$STATE_DIR/config/gateway-token")
        fi
        if [ -z "$HBGWTOKEN" ]; then
          HBGWTOKEN=$(openssl rand -hex 32 2>/dev/null || head -c 32 /dev/urandom | xxd -p -c 64)
          printf '%s' "$HBGWTOKEN" > "$STATE_DIR/config/gateway-token"
          chmod 600 "$STATE_DIR/config/gateway-token"
        fi
        # Create or update config — ALWAYS patch env block with API key
        if [ "$RUNTIME_CFG" = "openclaw" ] && command -v node >/dev/null 2>&1; then
          node - "$HBGWTOKEN" "$AKEY" "$SAFE_PROVIDER" <<'HBCONFIGEOF'
var fs = require('fs'), t = process.argv[2] || '', apiKey = process.argv[3], provider = process.argv[4];
var configPath = '/root/.openclaw/openclaw.json';
var config;
try { config = JSON.parse(fs.readFileSync(configPath, 'utf8')); } catch(e) { config = null; }
// Create fresh config if missing
if (!config) {
  var auth = t ? {mode:'token',token:t} : {mode:'none'};
  config = {gateway:{mode:'local',auth:auth},plugins:{allow:['osmoda-bridge','device-pair','memory-core','phone-control','talk-voice']},agents:{defaults:{compaction:{mode:'safeguard'}},list:[{id:'osmoda',default:true,name:'osModa',workspace:'/root/.openclaw/workspace-osmoda',agentDir:'/root/.openclaw/agents/osmoda/agent',model:'anthropic/claude-opus-4-6'},{id:'mobile',name:'osModa Mobile',workspace:'/root/.openclaw/workspace-mobile',agentDir:'/root/.openclaw/agents/mobile/agent',model:'anthropic/claude-sonnet-4-6'}]},bindings:[{agentId:'mobile',match:{channel:'telegram'}},{agentId:'mobile',match:{channel:'whatsapp'}}]};
}
// Always set compaction safeguard
if (!config.agents) config.agents = {};
if (!config.agents.defaults) config.agents.defaults = {};
config.agents.defaults.compaction = {mode:'safeguard'};
// Always patch env block with the API key — this is the key trick
if (!config.env) config.env = {};
if (provider === 'anthropic') {
  // OAuth tokens (sk-ant-oat) go in BOTH fields; regular API keys go in ANTHROPIC_API_KEY only
  var isOAuth = apiKey.startsWith('sk-ant-oat');
  config.env.ANTHROPIC_API_KEY = apiKey;
  if (isOAuth) {
    config.env.CLAUDE_CODE_OAUTH_TOKEN = apiKey;
  } else {
    delete config.env.CLAUDE_CODE_OAUTH_TOKEN;
  }
  delete config.env.OPENAI_API_KEY;
} else if (provider === 'openai') {
  config.env.OPENAI_API_KEY = apiKey;
  delete config.env.ANTHROPIC_API_KEY;
  delete config.env.CLAUDE_CODE_OAUTH_TOKEN;
}
fs.writeFileSync(configPath, JSON.stringify(config, null, 2));
HBCONFIGEOF
        fi
        # Write gateway env vars for daemon sockets
        cat > "$STATE_DIR/config/gateway-env" << 'GWENVEOF'
OSMODA_SOCKET=/run/osmoda/agentd.sock
OSMODA_KEYD_SOCKET=/run/osmoda/keyd.sock
OSMODA_WATCH_SOCKET=/run/osmoda/watch.sock
OSMODA_ROUTINES_SOCKET=/run/osmoda/routines.sock
OSMODA_VOICE_SOCKET=/run/osmoda/voice.sock
OSMODA_MESH_SOCKET=/run/osmoda/mesh.sock
OSMODA_MCPD_SOCKET=/run/osmoda/mcpd.sock
OSMODA_TEACHD_SOCKET=/run/osmoda/teachd.sock
GWENVEOF
        chmod 600 "$STATE_DIR/config/gateway-env"
        # Enable + restart gateway to pick up new key + config
        systemctl enable osmoda-gateway.service 2>/dev/null || true
        systemctl restart osmoda-gateway.service 2>/dev/null || true
        NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" '. + [$id]')
      fi
      ;;
    remove_api_key)
      # Remove API key + stop gateway
      rm -f "$STATE_DIR/config/api-key" "$STATE_DIR/config/env"
      if [ "$RUNTIME_CFG" = "openclaw" ]; then
        for _AGID in osmoda mobile; do
          rm -f "/root/.openclaw/agents/$_AGID/agent/auth-profiles.json"
        done
      fi
      systemctl stop osmoda-gateway.service 2>/dev/null || true
      systemctl disable osmoda-gateway.service 2>/dev/null || true
      NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" '. + [$id]')
      ;;
    connect_channel)
      ACHANNEL=$(echo "$ACTION_JSON" | jq -r '.channel' 2>/dev/null) || continue
      ATOKEN=$(echo "$ACTION_JSON" | jq -r '.token' 2>/dev/null) || continue
      # Extract allowed user IDs (JSON array → comma-separated)
      ALLOWED_USERS=$(echo "$ACTION_JSON" | jq -r '(.allowed_users // []) | join(",")' 2>/dev/null) || ALLOWED_USERS=""
      # Sanitize channel name to prevent path traversal
      ACHANNEL=$(echo "$ACHANNEL" | tr -cd 'a-z')
      if [ -n "$ACHANNEL" ] && [ "$ACHANNEL" != "null" ] && [ -n "$ATOKEN" ] && [ "$ATOKEN" != "null" ]; then
        # Decrypt token if encrypted
        if echo "$ATOKEN" | grep -q '^ENC:' && [ -n "$HBSECRET" ]; then
          ENC_IV=$(echo "$ATOKEN" | cut -d: -f2)
          ENC_TAG=$(echo "$ATOKEN" | cut -d: -f3)
          ENC_CT=$(echo "$ATOKEN" | cut -d: -f4)
          ATOKEN=$(node - "$HBSECRET" "$ENC_IV" "$ENC_TAG" "$ENC_CT" <<'DECEOF'
var c=require('crypto'),s=process.argv[2],iv=process.argv[3],tag=process.argv[4],ct=process.argv[5];
var dk=c.createHash('sha256').update(s).digest();
var d=c.createDecipheriv('aes-256-gcm',dk,Buffer.from(iv,'hex'));
d.setAuthTag(Buffer.from(tag,'hex'));
var pt=d.update(ct,'hex','utf8')+d.final('utf8');
process.stdout.write(pt);
DECEOF
          ) || true
          if [ -z "$ATOKEN" ]; then
            echo "Failed to decrypt channel token — skipping action $AID"
            continue
          fi
        fi
        # Save token to secrets file
        mkdir -p "$STATE_DIR/secrets"
        printf '%s' "$ATOKEN" > "$STATE_DIR/secrets/${ACHANNEL}-bot-token"
        chmod 600 "$STATE_DIR/secrets/${ACHANNEL}-bot-token"
        # Update config to add channel
        if [ "$RUNTIME_CFG" = "claude-code" ] && [ -f "$GATEWAY_CONFIG" ] && command -v node >/dev/null 2>&1; then
          node - "$ACHANNEL" "$STATE_DIR/secrets/${ACHANNEL}-bot-token" "$ALLOWED_USERS" <<'GWCHADDEOF'
var fs=require('fs'),ch=process.argv[2],tf=process.argv[3],au=process.argv[4]||'';
var config=JSON.parse(fs.readFileSync('/var/lib/osmoda/config/gateway.json','utf8'));
if(!config.telegram) config.telegram = {};
var allowList=au?au.split(',').filter(function(u){return u.trim()!=='';}):[];
config.telegram.botToken = fs.readFileSync(tf,'utf8').trim();
if(allowList.length>0) config.telegram.allowedUsers = allowList;
fs.writeFileSync('/var/lib/osmoda/config/gateway.json',JSON.stringify(config,null,2));
GWCHADDEOF
        elif command -v node >/dev/null 2>&1; then
          node - "$ACHANNEL" "$STATE_DIR/secrets/${ACHANNEL}-bot-token" "$ALLOWED_USERS" <<'CHADDEOF'
var fs=require('fs'),ch=process.argv[2],tf=process.argv[3],au=process.argv[4]||'';
var configPath='/root/.openclaw/openclaw.json';
var config;
try { config=JSON.parse(fs.readFileSync(configPath,'utf8')); } catch(e) {
  config={gateway:{mode:'local',auth:{mode:'none'}},plugins:{allow:['osmoda-bridge','device-pair','memory-core','phone-control','talk-voice']},agents:{list:[{id:'osmoda',default:true,name:'osModa',workspace:'/root/.openclaw/workspace-osmoda',agentDir:'/root/.openclaw/agents/osmoda/agent',model:'anthropic/claude-opus-4-6'},{id:'mobile',name:'osModa Mobile',workspace:'/root/.openclaw/workspace-mobile',agentDir:'/root/.openclaw/agents/mobile/agent',model:'anthropic/claude-sonnet-4-6'}]},bindings:[{agentId:'mobile',match:{channel:'telegram'}},{agentId:'mobile',match:{channel:'whatsapp'}}]};
}
if(!config.channels)config.channels={};
var allowList=au?au.split(',').filter(function(u){return /^\d+$/.test(u)}):[];
config.channels[ch]={enabled:true,tokenFile:tf,dmPolicy:allowList.length>0?'allowlist':'pairing',allowFrom:allowList.length>0?allowList:[]};
fs.writeFileSync(configPath,JSON.stringify(config,null,2));
CHADDEOF
        fi
        # Restart gateway to pick up new channel
        systemctl restart osmoda-gateway.service 2>/dev/null || true
        NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" '. + [$id]')
      fi
      ;;
    pairing_approve)
      # OpenClaw-only: approve channel pairing
      if [ "$RUNTIME_CFG" = "openclaw" ]; then
        ACHANNEL=$(echo "$ACTION_JSON" | jq -r '.channel' 2>/dev/null) || continue
        ACODE=$(echo "$ACTION_JSON" | jq -r '.code' 2>/dev/null) || continue
        if [ -n "$ACHANNEL" ] && [ "$ACHANNEL" != "null" ] && [ -n "$ACODE" ] && [ "$ACODE" != "null" ]; then
          SAFE_CODE=$(echo "$ACODE" | tr -cd 'A-Z0-9')
          SAFE_CHANNEL=$(echo "$ACHANNEL" | tr -cd 'a-z')
          openclaw pairing approve "$SAFE_CHANNEL" "$SAFE_CODE" 2>/dev/null || true
        fi
      fi
      NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" '. + [$id]')
      ;;
    disconnect_channel)
      ACHANNEL=$(echo "$ACTION_JSON" | jq -r '.channel' 2>/dev/null) || continue
      # Sanitize channel name to prevent path traversal
      ACHANNEL=$(echo "$ACHANNEL" | tr -cd 'a-z')
      if [ -n "$ACHANNEL" ] && [ "$ACHANNEL" != "null" ]; then
        # Remove token file
        rm -f "$STATE_DIR/secrets/${ACHANNEL}-bot-token"
        # Remove channel from config
        if [ "$RUNTIME_CFG" = "claude-code" ] && [ -f "$GATEWAY_CONFIG" ] && command -v node >/dev/null 2>&1; then
          node - "$ACHANNEL" <<'GWCHRMEOF'
var fs=require('fs'),ch=process.argv[2];
var config=JSON.parse(fs.readFileSync('/var/lib/osmoda/config/gateway.json','utf8'));
if(config.telegram) delete config.telegram;
fs.writeFileSync('/var/lib/osmoda/config/gateway.json',JSON.stringify(config,null,2));
GWCHRMEOF
        elif [ -f "/root/.openclaw/openclaw.json" ] && command -v node >/dev/null 2>&1; then
          node - "$ACHANNEL" <<'CHRMEOF'
var fs=require('fs'),ch=process.argv[2];
var config=JSON.parse(fs.readFileSync('/root/.openclaw/openclaw.json','utf8'));
if(config.channels&&config.channels[ch])delete config.channels[ch];
fs.writeFileSync('/root/.openclaw/openclaw.json',JSON.stringify(config,null,2));
CHRMEOF
        fi
        # Restart gateway to apply change
        systemctl restart osmoda-gateway.service 2>/dev/null || true
        NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" '. + [$id]')
      fi
      ;;
  esac
done

# If we completed any actions, send completions back immediately (don't wait 5 min)
if [ "$NEW_COMPLETED" != "[]" ] && [ -n "$NEW_COMPLETED" ]; then
  PAYLOAD2=$(jq -n \
    --arg oid "$OID" \
    --argjson completed "$NEW_COMPLETED" \
    '{order_id: $oid, status: "alive", setup_complete: true, openclaw_ready: '"$OC_READY"', gateway_ready: '"$OC_READY"', completed_actions: $completed}'
  )
  if [ -n "$HBSECRET" ]; then
    HB_TS2=$(date +%s000)
    SIG2=$(printf '%s:%s' "$OID" "$HB_TS2" | openssl dgst -sha256 -hmac "$HBSECRET" 2>/dev/null | awk '{print $NF}')
    curl -sf --max-time 15 -X POST "$CBURL" \
      -H "Content-Type: application/json" \
      -H "X-Heartbeat-Signature: $SIG2" \
      -H "X-Heartbeat-Timestamp: $HB_TS2" \
      -d "$PAYLOAD2" >/dev/null 2>&1
  else
    curl -sf --max-time 15 -X POST "$CBURL" \
      -H "Content-Type: application/json" \
      -d "$PAYLOAD2" >/dev/null 2>&1
  fi
  # Completions sent — clear the file
  echo "[]" > "$COMPLETED_FILE"
else
  echo "$NEW_COMPLETED" > "$COMPLETED_FILE"
fi
HBEOF
chmod +x "$INSTALL_DIR/bin/osmoda-heartbeat.sh"

cat > "$SYSTEMD_DIR/osmoda-heartbeat.service" <<EOF
[Unit]
Description=osModa Heartbeat (phones home to spawn.os.moda)
After=network-online.target osmoda-agentd.service
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=$INSTALL_DIR/bin/osmoda-heartbeat.sh
EOF

cat > "$SYSTEMD_DIR/osmoda-heartbeat.timer" <<EOF
[Unit]
Description=osModa Heartbeat Timer

[Timer]
OnBootSec=30
OnUnitActiveSec=5min
AccuracySec=30

[Install]
WantedBy=timers.target
EOF
fi

# On NixOS with read-only /etc, `systemctl enable` fails (can't create symlinks).
# Services will be started directly; osmoda-app-restore handles boot persistence.
svc_enable() { systemctl enable "$1" 2>/dev/null || true; }

systemctl daemon-reload
svc_enable osmoda-agentd.service
systemctl start osmoda-agentd.service

# Wait for agentd socket (30s timeout — builds can be slow to start)
log "Waiting for agentd socket..."
for i in $(seq 1 30); do
  if [ -S "$RUN_DIR/agentd.sock" ]; then break; fi
  sleep 1
done

if [ -S "$RUN_DIR/agentd.sock" ]; then
  log "agentd is running."
else
  warn "agentd socket not found after 30s. Check: journalctl -u osmoda-agentd"
fi

# Start all daemons
for svc in osmoda-keyd osmoda-watch osmoda-routines osmoda-mesh osmoda-mcpd osmoda-teachd osmoda-voice osmoda-egress osmoda-app-restore; do
  if [ -f "$SYSTEMD_DIR/${svc}.service" ]; then
    svc_enable "${svc}.service"
    systemctl start "${svc}.service"
  fi
done

# Always enable gateway (auto-start on reboot once key exists)
svc_enable osmoda-gateway.service
if [ -f "$STATE_DIR/config/api-key" ] || [ -f "$STATE_DIR/config/env" ]; then
  systemctl start osmoda-gateway.service
  log "Gateway ($RUNTIME) starting on port 18789..."
else
  log "Gateway enabled (will auto-start after API key is set and service started)."
fi

# Enable heartbeat timer if configured
if [ -f "$SYSTEMD_DIR/osmoda-heartbeat.timer" ]; then
  svc_enable osmoda-heartbeat.timer
  systemctl start osmoda-heartbeat.timer
  log "Heartbeat timer started (every 5 min)."
fi

# Pair device identity with OpenClaw (requires gateway to be running) — openclaw only
# Claude Code runtime uses simple token auth (no pairing needed)
if [ "$RUNTIME" = "openclaw" ]; then
IDENTITY_DIR="/root/.openclaw/identity"
if [ -f "$IDENTITY_DIR/device.json" ] && [ ! -f "$IDENTITY_DIR/device-auth.json" ] || \
   ([ -f "$IDENTITY_DIR/device-auth.json" ] && ! grep -q '"operator"' "$IDENTITY_DIR/device-auth.json" 2>/dev/null); then
  log "Pairing device identity with OpenClaw gateway..."
  # Wait for gateway to be ready
  for i in $(seq 1 10); do
    if curl -sf http://127.0.0.1:18789/health >/dev/null 2>&1; then break; fi
    sleep 2
  done
  if curl -sf http://127.0.0.1:18789/health >/dev/null 2>&1; then
    GATEWAY_TOKEN_VAL=""
    [ -f "$STATE_DIR/config/gateway-token" ] && GATEWAY_TOKEN_VAL=$(cat "$STATE_DIR/config/gateway-token")
    # Step 1: Connect to trigger pairing request
    DEVICE_ID=$(node -e "console.log(JSON.parse(require('fs').readFileSync('$IDENTITY_DIR/device.json')).deviceId)")
    cd /opt/openclaw && node -e "
const crypto=require('crypto'),fs=require('fs'),WebSocket=require('ws');
const id=JSON.parse(fs.readFileSync('$IDENTITY_DIR/device.json'));
const spkiDer=crypto.createPublicKey(id.publicKeyPem).export({type:'spki',format:'der'});
const rawKey=spkiDer.subarray(spkiDer.length-32);
const ws=new WebSocket('ws://127.0.0.1:18789',{headers:{origin:'http://127.0.0.1:18789'}});
let cid=null;
ws.on('error',()=>process.exit(0));
ws.on('message',(d)=>{
  const m=JSON.parse(d.toString());
  if(m.type==='event'&&m.event==='connect.challenge'){
    const n=m.payload?.nonce||m.nonce;cid=crypto.randomUUID();
    const now=Date.now(),s='operator.admin,operator.write,operator.read';
    const p='v3|'+id.deviceId+'|gateway-client|cli|operator|'+s+'|'+now+'|$GATEWAY_TOKEN_VAL|'+n+'|linux|';
    const sig=crypto.sign(null,Buffer.from(p),id.privateKeyPem).toString('base64url');
    ws.send(JSON.stringify({type:'req',id:cid,method:'connect',params:{
      minProtocol:3,maxProtocol:3,client:{id:'gateway-client',version:'1.0.0',platform:'linux',mode:'cli',instanceId:cid},
      role:'operator',scopes:['operator.admin','operator.write','operator.read'],caps:[],
      device:{id:id.deviceId,publicKey:rawKey.toString('base64url'),signature:sig,nonce:n,signedAt:now},
      auth:{token:'$GATEWAY_TOKEN_VAL'}
    }}));
  }
  if(m.type==='res'&&m.id===cid){
    if(m.ok){
      const sc=m.payload?.auth?.scopes||[],dt=m.payload?.auth?.deviceToken||'';
      console.log('[device] auto-paired, scopes:',sc.join(','));
      fs.writeFileSync('$IDENTITY_DIR/device-auth.json',JSON.stringify({version:1,deviceId:id.deviceId,
        tokens:{operator:{token:dt,role:'operator',scopes:sc,updatedAtMs:Date.now()}}},null,2),{mode:0o600});
    } else {
      console.log('[device] pairing request sent, needs approval');
    }
    ws.close();process.exit(0);
  }
});
setTimeout(()=>process.exit(0),10000);
" 2>&1

    # Step 2: Approve the pending pairing request via CLI
    if [ ! -f "$IDENTITY_DIR/device-auth.json" ] || ! grep -q '"operator.write"' "$IDENTITY_DIR/device-auth.json" 2>/dev/null; then
      log "Approving device pairing request..."
      export PATH="$OPENCLAW_DIR/node_modules/.bin:$PATH"
      # Get the pending request ID and approve it
      PENDING_INFO=$(openclaw devices list --json 2>/dev/null | node -e "
        const d=[];process.stdin.on('data',c=>d.push(c));
        process.stdin.on('end',()=>{
          try{const j=JSON.parse(Buffer.concat(d));
          const p=(j.pending||[]).find(p=>p.deviceId&&p.deviceId.startsWith('$DEVICE_ID'.slice(0,16)));
          if(p)console.log(p.requestId||p.id||'');
          }catch{}
        });
      " 2>/dev/null)
      if [ -n "$PENDING_INFO" ] && [ "$PENDING_INFO" != "" ]; then
        openclaw devices approve "$PENDING_INFO" 2>&1 || true
        log "Device approved. Reconnecting to get token..."
        # Step 3: Reconnect to get the device token after approval
        sleep 2
        cd /opt/openclaw && node -e "
const crypto=require('crypto'),fs=require('fs'),WebSocket=require('ws');
const id=JSON.parse(fs.readFileSync('$IDENTITY_DIR/device.json'));
const spkiDer=crypto.createPublicKey(id.publicKeyPem).export({type:'spki',format:'der'});
const rawKey=spkiDer.subarray(spkiDer.length-32);
const ws=new WebSocket('ws://127.0.0.1:18789',{headers:{origin:'http://127.0.0.1:18789'}});
let cid=null;
ws.on('error',()=>process.exit(0));
ws.on('message',(d)=>{
  const m=JSON.parse(d.toString());
  if(m.type==='event'&&m.event==='connect.challenge'){
    const n=m.payload?.nonce||m.nonce;cid=crypto.randomUUID();
    const now=Date.now(),s='operator.admin,operator.write,operator.read';
    const p='v3|'+id.deviceId+'|gateway-client|cli|operator|'+s+'|'+now+'|$GATEWAY_TOKEN_VAL|'+n+'|linux|';
    const sig=crypto.sign(null,Buffer.from(p),id.privateKeyPem).toString('base64url');
    ws.send(JSON.stringify({type:'req',id:cid,method:'connect',params:{
      minProtocol:3,maxProtocol:3,client:{id:'gateway-client',version:'1.0.0',platform:'linux',mode:'cli',instanceId:cid},
      role:'operator',scopes:['operator.admin','operator.write','operator.read'],caps:[],
      device:{id:id.deviceId,publicKey:rawKey.toString('base64url'),signature:sig,nonce:n,signedAt:now},
      auth:{token:'$GATEWAY_TOKEN_VAL'}
    }}));
  }
  if(m.type==='res'&&m.id===cid){
    const sc=m.payload?.auth?.scopes||[],dt=m.payload?.auth?.deviceToken||'';
    console.log('[device] paired with scopes:',sc.join(','));
    fs.writeFileSync('$IDENTITY_DIR/device-auth.json',JSON.stringify({version:1,deviceId:id.deviceId,
      tokens:{operator:{token:dt,role:'operator',scopes:sc,updatedAtMs:Date.now()}}},null,2),{mode:0o600});
    ws.close();process.exit(0);
  }
});
setTimeout(()=>process.exit(0),10000);
" 2>&1
      else
        warn "No pending pairing request found to approve"
      fi
    fi
    # Device pairing attempted above; failures are non-fatal (relay retries)
  else
    warn "Gateway not responding — device pairing skipped (relay will retry)"
  fi
fi
fi  # end RUNTIME=openclaw pairing block

# Enable WS relay if configured
if [ -f "$SYSTEMD_DIR/osmoda-ws-relay.service" ]; then
  svc_enable osmoda-ws-relay.service
  systemctl start osmoda-ws-relay.service
  log "WebSocket chat relay started."
fi
fi # end SKIP_SYSTEMD

# Final pass: ensure Hetzner password expiry is cleared (races with cloud-init)
passwd -d root 2>/dev/null || true
chage -I -1 -m 0 -M 99999 -E -1 root 2>/dev/null || true

# Nuclear fix: Hetzner cloud-init can re-expire password AFTER our install completes.
# Install a oneshot timer that clears expiry every 30s for the first 5 minutes.
cat > /opt/osmoda/bin/fix-password-expiry.sh << 'PWEOF'
#!/usr/bin/env bash
# Clear root password expiry — prevents "Password change required but no TTY"
passwd -d root 2>/dev/null || true
chage -I -1 -m 0 -M 99999 -E -1 root 2>/dev/null || true
PWEOF
chmod +x /opt/osmoda/bin/fix-password-expiry.sh

# Run it 10 times over 5 minutes via background loop, then self-destruct
(for i in $(seq 1 10); do
  sleep 30
  /opt/osmoda/bin/fix-password-expiry.sh
done) &

# ---------------------------------------------------------------------------
# Done!
# ---------------------------------------------------------------------------
report_progress "services" "done" "All services running"
report_progress "complete" "done" "osModa installed successfully"
echo ""
echo -e "${BOLD}  ╔══════════════════════════════════════════╗${NC}"
echo -e "${BOLD}  ║       ${GREEN}osModa installed successfully!${NC}${BOLD}      ║${NC}"
echo -e "${BOLD}  ╚══════════════════════════════════════════╝${NC}"
echo ""
info "Your computer now has a brain."
echo ""

if [ -f "$STATE_DIR/config/api-key" ]; then
  info "Chat with your system:"
  info "  Open http://localhost:18789 in your browser"
  info "  Or SSH tunnel: ssh -L 18789:localhost:18789 root@<this-ip>"
else
  info "Next step — set your Anthropic API key:"
  info ""
  info "  1. Write the env file:"
  info "     echo 'ANTHROPIC_API_KEY=sk-ant-api03-YOUR_KEY' > $STATE_DIR/config/env"
  info ""
  info "  2. Start the gateway:"
  info "     systemctl start osmoda-gateway"
fi

echo ""
info "Messaging channels (optional):"
info "  Telegram: Create a bot via @BotFather, save token to $STATE_DIR/secrets/telegram-bot-token"
info "            Then add to configuration.nix: services.osmoda.channels.telegram.enable = true;"
info "  Guide:    https://github.com/bolivian-peru/os-moda/blob/main/docs/CHANNELS.md"
echo ""
info "Useful commands:"
info "  curl -s --unix-socket $RUN_DIR/agentd.sock http://l/health | jq"
info "  journalctl -u osmoda-agentd -f"
info "  journalctl -u osmoda-gateway -f"
echo ""
info "Documentation: https://os.moda"
info "Report issues: https://github.com/bolivian-peru/os-moda/issues"
echo ""

# Fire immediate heartbeat on install completion (don't wait for timer)
# This sends full health data + processes any queued actions (API key, SSH keys, etc.)
if [ -n "$ORDER_ID" ] && [ -n "$CALLBACK_URL" ] && [ -x "$INSTALL_DIR/bin/osmoda-heartbeat.sh" ]; then
  log "Sending initial heartbeat to spawn.os.moda..."
  "$INSTALL_DIR/bin/osmoda-heartbeat.sh" 2>/dev/null || true
  # Run a second time after 10s to pick up any actions returned by the first heartbeat
  ( sleep 10 && "$INSTALL_DIR/bin/osmoda-heartbeat.sh" 2>/dev/null || true ) &
fi
