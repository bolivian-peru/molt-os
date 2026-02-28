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
#   1. Converts your server to NixOS (via nixos-infect) — optional
#   2. Installs Rust toolchain + builds agentd
#   3. Installs OpenClaw AI gateway
#   4. Sets up the osmoda-bridge plugin (72 system tools)
#   5. Installs agent identity + skills
#   6. Starts everything — agentd + OpenClaw
#   7. Opens the setup wizard at localhost:18789
#
# Supports: Ubuntu 22.04+, Debian 12+, existing NixOS
# Tested on: Hetzner Cloud, DigitalOcean, bare metal
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
# Parse arguments
# ---------------------------------------------------------------------------
SKIP_NIXOS=false
API_KEY=""
BRANCH="main"
ORDER_ID=""
CALLBACK_URL=""
HEARTBEAT_SECRET=""
PROVIDER_TYPE=""

while [[ $# -gt 0 ]]; do
  case $1 in
    --skip-nixos)        SKIP_NIXOS=true; shift ;;
    --api-key)           API_KEY="$2"; shift 2 ;;
    --branch)            BRANCH="$2"; shift 2 ;;
    --order-id)          ORDER_ID="$2"; shift 2 ;;
    --callback-url)      CALLBACK_URL="$2"; shift 2 ;;
    --heartbeat-secret)  HEARTBEAT_SECRET="$2"; shift 2 ;;
    --provider)          PROVIDER_TYPE="$2"; shift 2 ;;
    --help|-h)
      echo "osModa Installer v${VERSION}"
      echo ""
      echo "Usage: curl -fsSL <url> | bash -s -- [options]"
      echo ""
      echo "Options:"
      echo "  --skip-nixos          Don't install NixOS (use on existing NixOS systems)"
      echo "  --api-key KEY         Set API key (base64-encoded, skips setup wizard)"
      echo "  --branch NAME         Git branch to install (default: main)"
      echo "  --order-id UUID       Spawn order ID (set by spawn.os.moda)"
      echo "  --callback-url URL    Heartbeat callback URL (set by spawn.os.moda)"
      echo "  --heartbeat-secret S  HMAC secret for heartbeat authentication"
      echo "  --provider TYPE       AI provider: anthropic or openai (default: anthropic)"
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
  if [ -z "${ORDER_ID:-}" ] || [ -z "${CALLBACK_URL:-}" ]; then return 0; fi
  local BASE_URL="${CALLBACK_URL%/api/heartbeat}"
  curl -sf --max-time 10 -X POST "$BASE_URL/api/provision-progress" \
    -H "Content-Type: application/json" \
    -H "X-Heartbeat-Secret: ${HEARTBEAT_SECRET:-}" \
    -d "{\"order_id\":\"$ORDER_ID\",\"step\":\"$step\",\"status\":\"$step_status\",\"detail\":\"$detail\"}" \
    >/dev/null 2>&1 &
}

# Report errors on exit so dashboard shows failure
trap 'if [ $? -ne 0 ]; then report_progress "error" "error" "Install failed at line $LINENO (exit $?)"; wait; fi' EXIT

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------
report_progress "preflight" "started" "Running pre-flight checks"
log "Running pre-flight checks..."

if [ "$(id -u)" -ne 0 ]; then
  die "This installer must be run as root. Try: sudo bash"
fi

# Detect OS
if [ -f /etc/NIXOS ]; then
  OS_TYPE="nixos"
  log "Detected: NixOS"
  SKIP_NIXOS=true
elif [ -f /etc/os-release ]; then
  . /etc/os-release
  OS_TYPE="${ID:-unknown}"
  log "Detected: ${PRETTY_NAME:-$OS_TYPE}"
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
# Step 1: NixOS installation (via nixos-infect)
# ---------------------------------------------------------------------------
# Step 1: NixOS conversion (only when explicitly requested — NOT during spawn deploys)
# Spawn cloud-init always passes --skip-nixos. This block only runs for manual installs
# that want to convert Ubuntu/Debian to NixOS (interactive use case).
if [ "$SKIP_NIXOS" = false ]; then
  echo ""
  warn "Step 1: NixOS Installation"
  warn "This will REPLACE your current OS with NixOS."
  warn "Your server will reboot. SSH back in after ~3 minutes."
  echo ""

  if [ "$OS_TYPE" = "ubuntu" ] || [ "$OS_TYPE" = "debian" ]; then
    log "Installing NixOS via nixos-infect..."
    log "This takes 5-10 minutes. The server will reboot automatically."
    echo ""

    # Auto-detect cloud provider from metadata endpoints
    PROVIDER="generic"
    if curl -sf -m 2 http://169.254.169.254/hetzner/v1/metadata >/dev/null 2>&1; then
      PROVIDER="hetznercloud"
    elif curl -sf -m 2 http://169.254.169.254/metadata/v1/ >/dev/null 2>&1; then
      PROVIDER="digitalocean"
    elif curl -sf -m 2 http://169.254.169.254/latest/meta-data/ >/dev/null 2>&1; then
      PROVIDER="ec2"
    fi
    log "Detected cloud provider: $PROVIDER"

    if curl -fsSL https://raw.githubusercontent.com/elitak/nixos-infect/master/nixos-infect \
      | NIX_CHANNEL=nixos-unstable PROVIDER="$PROVIDER" bash -x; then
      # If we get here, infect didn't reboot (shouldn't happen)
      warn "nixos-infect complete. Please reboot and re-run this script with --skip-nixos"
      exit 0
    else
      error "nixos-infect failed (exit code $?). Falling through to install on current OS."
      warn "osModa will install on $OS_TYPE instead. NixOS conversion can be retried later."
      # Don't exit — continue installing daemons on the existing OS
    fi
  else
    warn "NixOS installation not supported for $OS_TYPE."
    warn "Continuing install on $OS_TYPE..."
    # Don't exit — install daemons on whatever OS is present
  fi
fi

# ---------------------------------------------------------------------------
# Step 2: Install dependencies
# ---------------------------------------------------------------------------
report_progress "preflight" "done" "$OS_TYPE $ARCH"
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
  # NixOS: install via nix-env
  for pkg in gcc gnumake pkg-config sqlite openssl cmake jq; do
    if ! nix-env -q "$pkg" &>/dev/null; then
      nix-env -iA "nixos.$pkg" 2>/dev/null || true
    fi
  done
elif [ "$OS_TYPE" = "ubuntu" ] || [ "$OS_TYPE" = "debian" ]; then
  log "Installing build dependencies for Ubuntu/Debian..."
  apt-get update -qq
  apt-get install -y -qq build-essential gcc g++ cmake pkg-config \
    libsqlite3-dev libssl-dev curl jq 2>&1 | tail -3
fi

# Ensure Rust toolchain
if ! command -v cargo &>/dev/null; then
  log "Installing Rust toolchain..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  export PATH="$HOME/.cargo/bin:$PATH"
fi
export PATH="$HOME/.cargo/bin:$PATH"

# Ensure Node.js for OpenClaw
if ! command -v node &>/dev/null; then
  if command -v nix-env &>/dev/null; then
    nix-env -iA nixos.nodejs_22
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
if ! cargo build --release --workspace 2>&1 | tee "$BUILD_LOG"; then
  error "Build failed. Full output:"
  cat "$BUILD_LOG"
  rm -f "$BUILD_LOG"
  die "Cargo build failed. See errors above."
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

# ---------------------------------------------------------------------------
# Step 5: Install OpenClaw
# ---------------------------------------------------------------------------
report_progress "build" "done" "All daemons compiled"
report_progress "openclaw" "started" "Installing OpenClaw AI gateway"
log "Step 5: Installing OpenClaw AI gateway..."

if ! command -v npm &>/dev/null; then
  die "npm is required but not found. Install Node.js (>= 18) and retry."
fi

if ! command -v openclaw &>/dev/null; then
  mkdir -p "$OPENCLAW_DIR"
  cd "$OPENCLAW_DIR"
  if [ ! -f package.json ]; then
    npm init -y >/dev/null 2>&1
  fi
  npm install openclaw 2>&1 | tail -3 || die "Failed to install OpenClaw via npm. Check network and npm permissions."
  ln -sf "$OPENCLAW_DIR/node_modules/.bin/openclaw" /usr/local/bin/openclaw 2>/dev/null || true
  # NixOS: add to profile if /usr/local/bin doesn't work
  echo "export PATH=\"$OPENCLAW_DIR/node_modules/.bin:\$PATH\"" >> /etc/profile.d/osmoda.sh
  export PATH="$OPENCLAW_DIR/node_modules/.bin:$PATH"
fi

log "OpenClaw installed."

# ---------------------------------------------------------------------------
# Step 6: Set up osmoda-bridge plugin
# ---------------------------------------------------------------------------
report_progress "openclaw" "done" "OpenClaw installed"
report_progress "bridge" "started" "Installing 72-tool bridge plugin"
log "Step 6: Setting up osmoda-bridge plugin..."

PLUGIN_SRC="$INSTALL_DIR/packages/osmoda-bridge"
PLUGIN_DST="/root/.openclaw/extensions/osmoda-bridge"

# Copy plugin to OpenClaw extensions (chown root — OpenClaw blocks non-root plugins)
mkdir -p /root/.openclaw/extensions
rm -rf "$PLUGIN_DST"
cp -r "$PLUGIN_SRC" "$PLUGIN_DST"
chown -R root:root "$PLUGIN_DST"

log "Bridge plugin installed with 72 system tools."

# ---------------------------------------------------------------------------
# Step 7: Multi-agent workspaces + skills (OpenClaw multi-agent routing)
# ---------------------------------------------------------------------------
report_progress "bridge" "done" "72 tools registered"
report_progress "workspaces" "started" "Setting up agent workspaces + skills"
log "Step 7: Setting up multi-agent workspaces..."

# OpenClaw multi-agent layout:
#   ~/.openclaw/workspace-osmoda/  — main agent (Opus, full access)
#   ~/.openclaw/workspace-mobile/  — mobile agent (Sonnet, read-only)
#   ~/.openclaw/agents/<id>/agent/ — per-agent state + auth
#   ~/.openclaw/agents/<id>/sessions/ — per-agent sessions
OC_BASE="/root/.openclaw"
WS_OSMODA="$OC_BASE/workspace-osmoda"
WS_MOBILE="$OC_BASE/workspace-mobile"

mkdir -p "$WORKSPACE_DIR" "$WS_OSMODA" "$WS_MOBILE"
mkdir -p "$OC_BASE/agents/osmoda/agent" "$OC_BASE/agents/osmoda/sessions"
mkdir -p "$OC_BASE/agents/mobile/agent" "$OC_BASE/agents/mobile/sessions"

# --- Main agent (osmoda): full templates + all skills ---
for tpl in AGENTS.md SOUL.md TOOLS.md IDENTITY.md USER.md HEARTBEAT.md; do
  if [ -f "$INSTALL_DIR/templates/$tpl" ]; then
    cp "$INSTALL_DIR/templates/$tpl" "$WORKSPACE_DIR/$tpl"
    cp "$INSTALL_DIR/templates/$tpl" "$WS_OSMODA/$tpl"
  fi
done

if [ -d "$INSTALL_DIR/skills" ]; then
  mkdir -p "$WORKSPACE_DIR/skills" "$WS_OSMODA/skills"
  cp -r "$INSTALL_DIR/skills/"* "$WORKSPACE_DIR/skills/" 2>/dev/null || true
  cp -r "$INSTALL_DIR/skills/"* "$WS_OSMODA/skills/" 2>/dev/null || true
fi

# --- Mobile agent: mobile-specific templates + monitoring skills only ---
if [ -d "$INSTALL_DIR/templates/agents/mobile" ]; then
  cp "$INSTALL_DIR/templates/agents/mobile/AGENTS.md" "$WS_MOBILE/AGENTS.md"
  cp "$INSTALL_DIR/templates/agents/mobile/SOUL.md" "$WS_MOBILE/SOUL.md"
fi
# Share TOOLS.md and IDENTITY.md from main templates
for tpl in TOOLS.md IDENTITY.md USER.md; do
  if [ -f "$INSTALL_DIR/templates/$tpl" ]; then
    cp "$INSTALL_DIR/templates/$tpl" "$WS_MOBILE/$tpl"
  fi
done

# Mobile skills: read-only monitoring subset
MOBILE_SKILLS="system-monitor service-explorer network-manager system-packages file-manager"
if [ -d "$INSTALL_DIR/skills" ]; then
  mkdir -p "$WS_MOBILE/skills"
  for skill in $MOBILE_SKILLS; do
    if [ -d "$INSTALL_DIR/skills/$skill" ]; then
      cp -r "$INSTALL_DIR/skills/$skill" "$WS_MOBILE/skills/$skill"
    fi
  done
fi

# Create state directories with secure permissions
mkdir -p "$STATE_DIR"/{memory,ledger,config,keyd/keys,watch,routines,mesh,mcp,teachd,apps}
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

# ---------------------------------------------------------------------------
# Step 8: Set up API key (if provided) or prep setup wizard
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
  else
    printf 'ANTHROPIC_API_KEY=%s\n' "$DECODED_KEY" > "$STATE_DIR/config/env"
  fi
  chmod 600 "$STATE_DIR/config/env"

  # Write OpenClaw auth-profiles.json for BOTH agents (shared API key)
  for AGENT_ID in osmoda mobile; do
    mkdir -p "/root/.openclaw/agents/$AGENT_ID/agent"
    if command -v node &>/dev/null; then
      if [ "$EFFECTIVE_PROVIDER" = "openai" ]; then
        node -e "
          const fs = require('fs');
          const key = process.argv[1];
          const agentId = process.argv[2];
          const auth = { type: 'api_key', provider: 'openai', key: key };
          fs.writeFileSync('/root/.openclaw/agents/' + agentId + '/agent/auth-profiles.json', JSON.stringify(auth, null, 2));
        " "$DECODED_KEY" "$AGENT_ID"
      else
        node -e "
          const fs = require('fs');
          const key = process.argv[1];
          const agentId = process.argv[2];
          const isOAuth = key.startsWith('sk-ant-oat');
          const auth = isOAuth
            ? { type: 'token', provider: 'anthropic', token: key }
            : { type: 'api_key', provider: 'anthropic', key: key };
          fs.writeFileSync('/root/.openclaw/agents/' + agentId + '/agent/auth-profiles.json', JSON.stringify(auth, null, 2));
        " "$DECODED_KEY" "$AGENT_ID"
      fi
    fi
  done

  # Generate gateway token for WS relay auth
  GATEWAY_TOKEN=$(openssl rand -hex 32 2>/dev/null || head -c 32 /dev/urandom | xxd -p -c 64)
  printf '%s' "$GATEWAY_TOKEN" > "$STATE_DIR/config/gateway-token"
  chmod 600 "$STATE_DIR/config/gateway-token"
  log "Generated gateway token for relay auth"

  # Generate multi-agent OpenClaw config (JSON5)
  if command -v node &>/dev/null; then
    node -e "
      const fs = require('fs');
      const gwToken = process.argv[1] || '';
      const config = {
        gateway: { mode: 'local', auth: gwToken ? { mode: 'token', token: gwToken } : { mode: 'none' } }," "$GATEWAY_TOKEN"
        plugins: { allow: ['osmoda-bridge', 'device-pair', 'memory-core', 'phone-control', 'talk-voice'] },
        agents: {
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
              model: 'anthropic/claude-sonnet-4-6',
              tools: {
                deny: [
                  'shell_exec', 'file_write',
                  'safety_panic', 'safety_rollback',
                  'app_deploy', 'app_remove', 'app_stop', 'app_restart',
                  'wallet_create', 'wallet_send', 'wallet_sign', 'wallet_delete',
                  'safe_switch_begin', 'safe_switch_commit', 'safe_switch_rollback',
                  'watcher_add', 'routine_add',
                  'mesh_invite_create', 'mesh_invite_accept', 'mesh_peer_disconnect',
                  'mesh_room_create', 'mesh_room_join',
                  'mcp_server_start', 'mcp_server_stop', 'mcp_server_restart',
                  'teach_knowledge_create', 'teach_optimize_suggest', 'teach_optimize_apply',
                  'incident_create', 'incident_step',
                  'backup_create'
                ]
              }
            }
          ]
        },
        bindings: [
          { agentId: 'mobile', match: { channel: 'telegram' } },
          { agentId: 'mobile', match: { channel: 'whatsapp' } }
        ]
      };
      fs.writeFileSync('/root/.openclaw/openclaw.json', JSON.stringify(config, null, 2));
    "
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
  log "Step 8: No API key provided — setup wizard will run on port 18789."
  info "After install, open http://localhost:18789 to enter your API key."
fi

# ---------------------------------------------------------------------------
# Step 9: Create and start systemd services
# ---------------------------------------------------------------------------
report_progress "apikey" "done" "Auth profiles written"
report_progress "services" "started" "Starting 9 daemons + OpenClaw gateway"
log "Step 9: Starting services..."

if [ "$OS_TYPE" = "nixos" ]; then
  # On NixOS, the recommended path is the osmoda.nix NixOS module.
  # But for install.sh bootstrap (e.g. fresh cloud server), write real systemd
  # unit files to /etc/systemd/system/ — they work on NixOS alongside the module.
  log "NixOS detected. Writing systemd unit files for persistent daemon management..."
  log "For production use, prefer: services.osmoda.enable = true in configuration.nix"
fi

SKIP_SYSTEMD=false
SYSTEMD_DIR="/etc/systemd/system"

mkdir -p "$RUN_DIR" "$STATE_DIR"
mkdir -p "$STATE_DIR"/{keyd/keys,watch,routines,mesh,config}
chmod 700 "$STATE_DIR/keyd" "$STATE_DIR/keyd/keys" "$STATE_DIR/mesh"

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
ExecStartPre=/bin/mkdir -p $RUN_DIR
ExecStartPre=/bin/mkdir -p $STATE_DIR

[Install]
WantedBy=multi-user.target
EOF

# OpenClaw gateway service
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
ExecStart=$INSTALL_DIR/bin/osmoda-mesh --socket $RUN_DIR/mesh.sock --data-dir $STATE_DIR/mesh --agentd-socket $RUN_DIR/agentd.sock --listen-port 18800
Restart=always
RestartSec=5
Environment=RUST_LOG=info
[Install]
WantedBy=multi-user.target
EOF

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

# app-restore (restores managed apps on boot)
cat > "$SYSTEMD_DIR/osmoda-app-restore.service" <<'AREOF'
[Unit]
Description=osModa App Process Restore
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=oneshot
RemainAfterExit=yes
ExecStart=/bin/bash -c '\
  REGISTRY="/var/lib/osmoda/apps/registry.json"; \
  if [ ! -f "$REGISTRY" ]; then exit 0; fi; \
  for APP_NAME in $(jq -r ".apps | to_entries[] | select(.value.status == \"running\") | .key" "$REGISTRY" 2>/dev/null); do \
    COMMAND=$(jq -r --arg n "$APP_NAME" ".apps[\$n].command" "$REGISTRY"); \
    RESTART=$(jq -r --arg n "$APP_NAME" ".apps[\$n].restart_policy // \"on-failure\"" "$REGISTRY"); \
    WORKDIR=$(jq -r --arg n "$APP_NAME" ".apps[\$n].working_dir // empty" "$REGISTRY"); \
    MEMMAX=$(jq -r --arg n "$APP_NAME" ".apps[\$n].memory_max // empty" "$REGISTRY"); \
    CPUQUOTA=$(jq -r --arg n "$APP_NAME" ".apps[\$n].cpu_quota // empty" "$REGISTRY"); \
    USER=$(jq -r --arg n "$APP_NAME" ".apps[\$n].user // empty" "$REGISTRY"); \
    SAFE_NAME=$(echo "$APP_NAME" | tr -cd "a-zA-Z0-9_-"); \
    UNIT="osmoda-app-$SAFE_NAME"; \
    SYSARGS=(--unit "$UNIT" --service-type=simple "--property=Restart=$RESTART" --property=StartLimitIntervalSec=0 --property=RestartSec=3); \
    if [ -n "$USER" ]; then SYSARGS+=("--uid=$USER"); else SYSARGS+=("--property=DynamicUser=yes"); fi; \
    [ -n "$WORKDIR" ] && SYSARGS+=("--working-directory=$WORKDIR"); \
    [ -n "$MEMMAX" ] && SYSARGS+=("--property=MemoryMax=$MEMMAX"); \
    [ -n "$CPUQUOTA" ] && SYSARGS+=("--property=CPUQuota=$CPUQUOTA"); \
    SYSARGS+=("--" "$COMMAND"); \
    echo "Restoring app: $APP_NAME (unit: $UNIT)"; \
    systemd-run "${SYSARGS[@]}" || echo "Failed to restore $APP_NAME"; \
  done'

[Install]
WantedBy=multi-user.target
AREOF

# WebSocket relay (bridges dashboard chat to OpenClaw gateway)
if [ -n "$ORDER_ID" ] && [ -n "$CALLBACK_URL" ]; then

cat > "$INSTALL_DIR/bin/osmoda-ws-relay.js" <<'WSEOF'
#!/usr/bin/env node
// osModa WS Relay — bridges spawn.os.moda dashboard chat to local OpenClaw gateway.
// Handles OpenClaw's protocol-v3 connect handshake, then translates between
// simple dashboard JSON and OpenClaw's RPC wire format.
const WebSocket = require("ws");
const fs = require("fs");
const crypto = require("crypto");

const STATE_DIR = "/var/lib/osmoda";
const RECONNECT_DELAY = 5000;
const OC_URL = "ws://127.0.0.1:18789";

function readConfig(name) {
  try { return fs.readFileSync(`${STATE_DIR}/config/${name}`, "utf8").trim(); }
  catch { return ""; }
}

function uid() { return crypto.randomUUID(); }

function connect() {
  const orderId = readConfig("order-id");
  const callbackUrl = readConfig("callback-url");
  const secret = readConfig("heartbeat-secret");
  if (!orderId || !callbackUrl || !secret) {
    console.error("[ws-relay] missing config, retrying in 30s...");
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
  let ocReady = false;       // true after OpenClaw connect handshake
  let connectId = null;      // pending connect request id
  let sessionKey = "spawn-" + orderId.slice(0, 8);
  let pendingChat = {};      // id → upstream tracking
  let instanceId = uid();

  function sendOcConnect() {
    connectId = uid();
    const token = readConfig("gateway-token");
    const msg = {
      type: "req", id: connectId, method: "connect",
      params: {
        minProtocol: 3, maxProtocol: 3,
        client: {
          id: "gateway-client", version: "1.0.0",
          platform: "linux", mode: "webchat", instanceId: instanceId
        },
        role: "operator",
        scopes: ["operator.admin"],
        caps: [],
        userAgent: "osmoda-ws-relay/1.0", locale: "en"
      }
    };
    if (token) msg.params.auth = { token: token };
    local.send(JSON.stringify(msg));
  }

  upstream.on("open", () => {
    console.log("[ws-relay] connected to spawn server");
    local = new WebSocket(OC_URL, { headers: { origin: "http://127.0.0.1:18789" } });

    local.on("open", () => {
      console.log("[ws-relay] connected to OpenClaw, handshaking...");
      sendOcConnect();
    });

    local.on("message", (data) => {
      const str = data.toString();
      let msg;
      try { msg = JSON.parse(str); } catch { return; }

      // Handle connect handshake
      if (!ocReady) {
        if (msg.type === "event" && msg.event === "connect.challenge") {
          // Challenge is informational — store nonce for future reconnects.
          // Do NOT re-send connect; the response to our original request follows.
          console.log("[ws-relay] got challenge (storing nonce for future use)");
          return;
        }
        if (msg.type === "res" && msg.id === connectId) {
          if (msg.ok) {
            ocReady = true;
            console.log("[ws-relay] OpenClaw handshake complete");
            if (upstream.readyState === WebSocket.OPEN) {
              upstream.send(JSON.stringify({ type: "status", openclaw_connected: true }));
            }
          } else {
            console.error("[ws-relay] OpenClaw connect rejected:", JSON.stringify(msg.error));
            local.close();
          }
          return;
        }
        // Other pre-handshake messages — ignore
        return;
      }

      // Post-handshake: translate OpenClaw events to dashboard format
      if (msg.type === "event") {
        // Forward relevant events to dashboard
        if (upstream.readyState === WebSocket.OPEN) {
          upstream.send(JSON.stringify(msg));
        }
        return;
      }

      // Response to a chat.send or other request
      if (msg.type === "res") {
        if (upstream.readyState === WebSocket.OPEN) {
          upstream.send(JSON.stringify(msg));
        }
        delete pendingChat[msg.id];
        return;
      }
    });

    local.on("close", () => {
      console.log("[ws-relay] OpenClaw disconnected, reconnecting...");
      ocReady = false;
      upstream.close();
    });

    local.on("error", (err) => {
      console.error("[ws-relay] OpenClaw error:", err.message);
    });
  });

  upstream.on("message", (data) => {
    if (!local || local.readyState !== WebSocket.OPEN || !ocReady) return;
    const str = data.toString();
    let msg;
    try { msg = JSON.parse(str); } catch { return; }

    // Dashboard sends simple chat: { type: "chat", text: "..." }
    // Translate to OpenClaw RPC: { type: "req", method: "chat.send", params: {...} }
    if (msg.type === "chat" && msg.text) {
      const reqId = uid();
      pendingChat[reqId] = true;
      local.send(JSON.stringify({
        type: "req", id: reqId, method: "chat.send",
        params: { text: msg.text, sessionKey: sessionKey }
      }));
      return;
    }

    // Dashboard sends chat abort: { type: "chat_abort" }
    if (msg.type === "chat_abort") {
      local.send(JSON.stringify({
        type: "req", id: uid(), method: "chat.abort",
        params: { sessionKey: sessionKey }
      }));
      return;
    }

    // Dashboard sends chat history request: { type: "chat_history" }
    if (msg.type === "chat_history") {
      local.send(JSON.stringify({
        type: "req", id: uid(), method: "chat.history",
        params: { sessionKey: sessionKey }
      }));
      return;
    }

    // Pass through any raw OpenClaw protocol messages
    if (msg.type === "req") {
      local.send(str);
      return;
    }
  });

  upstream.on("close", () => {
    console.log("[ws-relay] spawn disconnected, reconnecting...");
    ocReady = false;
    if (local) local.close();
    setTimeout(connect, RECONNECT_DELAY);
  });

  upstream.on("error", (err) => {
    console.error("[ws-relay] spawn error:", err.message);
  });
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
ExecStart=/usr/bin/env node $INSTALL_DIR/bin/osmoda-ws-relay.js
Restart=always
RestartSec=5
Environment=NODE_PATH=$OPENCLAW_DIR/node_modules
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

OID=$(cat "$STATE_DIR/config/order-id" 2>/dev/null) || exit 0
CBURL=$(cat "$STATE_DIR/config/callback-url" 2>/dev/null) || exit 0
HBSECRET=$(cat "$STATE_DIR/config/heartbeat-secret" 2>/dev/null || echo "")

# Collect health from agentd (5s timeout to prevent hangs)
HEALTH=$(curl -sf --max-time 5 --unix-socket "$RUN_DIR/agentd.sock" http://l/health 2>/dev/null || echo "{}")
CPU=$(echo "$HEALTH" | grep -o '"cpu":[0-9.]*' | head -1 | cut -d: -f2)
RAM=$(echo "$HEALTH" | grep -o '"ram":[0-9.]*' | head -1 | cut -d: -f2)
DISK=$(echo "$HEALTH" | grep -o '"disk":[0-9.]*' | head -1 | cut -d: -f2)
UPTIME=$(echo "$HEALTH" | grep -o '"uptime":[0-9.]*' | head -1 | cut -d: -f2)
OC_READY=$(systemctl is-active osmoda-gateway.service 2>/dev/null | grep -q "^active$" && echo true || echo false)

# Build completed_actions from previous heartbeat
COMPLETED_FILE="$STATE_DIR/config/completed-actions"
COMPLETED_JSON="[]"
if [ -f "$COMPLETED_FILE" ] && [ -s "$COMPLETED_FILE" ]; then
  COMPLETED_JSON=$(cat "$COMPLETED_FILE")
fi

# Collect agent instances
AGENTS_JSON="[]"
if [ -d /root/.openclaw/agents ]; then
  for agent_dir in /root/.openclaw/agents/*/; do
    [ -d "$agent_dir" ] || continue
    ANAME=$(basename "$agent_dir")
    ASTATUS="stopped"
    if systemctl is-active osmoda-gateway.service >/dev/null 2>&1; then
      ASTATUS="running"
    fi
    AGENTS_JSON=$(echo "$AGENTS_JSON" | jq --arg name "$ANAME" --arg status "$ASTATUS" \
      '. + [{name: $name, status: $status}]')
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
MESH_PEERS_SLIM=$(echo "$MESH_PEERS" | jq '[.[] | {id: .id, label: .label, state: (.connection_state // "unknown")}]' 2>/dev/null || echo "[]")

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
  '{order_id: $oid, status: "alive", setup_complete: true, openclaw_ready: $oc_ready, health: {cpu: $cpu, ram: $ram, disk: $disk, uptime: $uptime}, completed_actions: $completed, agents: $agents, daemon_health: $daemon_health, mesh_identity: $mesh_identity, mesh_peers: $mesh_peers}'
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

# Clear completed actions after successful send
echo "[]" > "$COMPLETED_FILE"

# Process pending actions from response
if ! command -v jq &>/dev/null; then exit 0; fi

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
      INVITE_RESULT=$(curl -sf --max-time 10 --unix-socket "$RUN_DIR/mesh.sock" \
        -X POST http://l/invite/create \
        -H "Content-Type: application/json" \
        -d '{"ttl_secs": 3600}' 2>/dev/null)
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
        curl -sf --max-time 10 --unix-socket "$RUN_DIR/mesh.sock" \
          -X POST http://l/invite/accept \
          -H "Content-Type: application/json" \
          -d "$ACCEPT_BODY" 2>/dev/null || true
        NEW_COMPLETED=$(echo "$NEW_COMPLETED" | jq --arg id "$AID" '. + [$id]')
      fi
      ;;
    mesh_peer_disconnect)
      # Disconnect a mesh peer
      PEER_ID=$(echo "$ACTION_JSON" | jq -r '.peer_instance_id' 2>/dev/null)
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
            AKEY=$(node -e "
              var c=require('crypto'),s=process.argv[1],iv=process.argv[2],tag=process.argv[3],ct=process.argv[4];
              var dk=c.createHash('sha256').update(s).digest();
              var d=c.createDecipheriv('aes-256-gcm',dk,Buffer.from(iv,'hex'));
              d.setAuthTag(Buffer.from(tag,'hex'));
              var pt=d.update(ct,'hex','utf8')+d.final('utf8');
              process.stdout.write(pt);
            " "$HBSECRET" "$ENC_IV" "$ENC_TAG" "$ENC_CT" 2>/dev/null) || true
          fi
          if [ -z "$AKEY" ]; then
            echo "Failed to decrypt API key — skipping action $AID"
            continue
          fi
        fi
        # Update stored API key
        printf '%s\n' "$AKEY" > "$STATE_DIR/config/api-key"
        chmod 600 "$STATE_DIR/config/api-key"
        # Update env file
        if [ "$APROVIDER" = "openai" ]; then
          printf 'OPENAI_API_KEY=%s\n' "$AKEY" > "$STATE_DIR/config/env"
        else
          printf 'ANTHROPIC_API_KEY=%s\n' "$AKEY" > "$STATE_DIR/config/env"
        fi
        chmod 600 "$STATE_DIR/config/env"
        # Update auth-profiles.json for all agents
        SAFE_PROVIDER="anthropic"
        if [ "$APROVIDER" = "openai" ]; then SAFE_PROVIDER="openai"; fi
        for _AGID in osmoda mobile; do
          mkdir -p "/root/.openclaw/agents/$_AGID/agent"
          jq -n --arg type "api_key" --arg provider "$SAFE_PROVIDER" --arg key "$AKEY" \
            '{type: $type, provider: $provider, key: $key}' \
            > "/root/.openclaw/agents/$_AGID/agent/auth-profiles.json"
        done
        # Ensure OpenClaw gateway config exists (may not if no key at install time)
        if [ ! -f "/root/.openclaw/openclaw.json" ] && command -v node >/dev/null 2>&1; then
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
          node -e "
            var fs=require('fs'),t=process.argv[1]||'';
            var auth=t?{mode:'token',token:t}:{mode:'none'};
            var config={gateway:{mode:'local',auth:auth},plugins:{allow:['osmoda-bridge','device-pair','memory-core','phone-control','talk-voice']},agents:{list:[{id:'osmoda',default:true,name:'osModa',workspace:'/root/.openclaw/workspace-osmoda',agentDir:'/root/.openclaw/agents/osmoda/agent',model:'anthropic/claude-opus-4-6'},{id:'mobile',name:'osModa Mobile',workspace:'/root/.openclaw/workspace-mobile',agentDir:'/root/.openclaw/agents/mobile/agent',model:'anthropic/claude-sonnet-4-6',tools:{deny:['shell_exec','file_write','safety_panic','safety_rollback','app_deploy','app_remove','app_stop','app_restart','wallet_create','wallet_send','wallet_sign','wallet_delete','safe_switch_begin','safe_switch_commit','safe_switch_rollback','watcher_add','routine_add','mesh_invite_create','mesh_invite_accept','mesh_peer_disconnect','mesh_room_create','mesh_room_join','mcp_server_start','mcp_server_stop','mcp_server_restart','teach_knowledge_create','teach_optimize_suggest','teach_optimize_apply','incident_create','incident_step','backup_create']}}]},bindings:[{agentId:'mobile',match:{channel:'telegram'}},{agentId:'mobile',match:{channel:'whatsapp'}}]};
            fs.writeFileSync('/root/.openclaw/openclaw.json',JSON.stringify(config,null,2));
          " "$HBGWTOKEN" 2>/dev/null
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
  esac
done

# Store completed action IDs for next heartbeat
echo "$NEW_COMPLETED" > "$COMPLETED_FILE"
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

systemctl daemon-reload
systemctl enable osmoda-agentd.service
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
    systemctl enable "${svc}.service"
    systemctl start "${svc}.service"
  fi
done

# Start OpenClaw if API key is configured
if [ -f "$STATE_DIR/config/api-key" ]; then
  systemctl enable osmoda-gateway.service
  systemctl start osmoda-gateway.service
  log "OpenClaw gateway starting on port 18789..."
else
  log "OpenClaw will start after you enter your API key."
fi

# Enable heartbeat timer if configured
if [ -f "$SYSTEMD_DIR/osmoda-heartbeat.timer" ]; then
  systemctl enable osmoda-heartbeat.timer
  systemctl start osmoda-heartbeat.timer
  log "Heartbeat timer started (every 5 min)."
fi

# Enable WS relay if configured
if [ -f "$SYSTEMD_DIR/osmoda-ws-relay.service" ]; then
  systemctl enable osmoda-ws-relay.service
  systemctl start osmoda-ws-relay.service
  log "WebSocket chat relay started."
fi
fi # end SKIP_SYSTEMD

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
  info "Next step — enter your Anthropic API key:"
  info "  Option 1: Open http://localhost:18789 (setup wizard)"
  info "  Option 2: Run: osmoda-setup"
  info "  Option 3: echo 'sk-ant-...' > $STATE_DIR/config/api-key"
fi

echo ""
info "Messaging channels (optional — requires OpenClaw channel support):"
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
