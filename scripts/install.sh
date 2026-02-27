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
#   4. Sets up the osmoda-bridge plugin (66 system tools)
#   5. Installs agent identity + skills
#   6. Starts everything — agentd + OpenClaw
#   7. Opens the setup wizard at localhost:18789
#
# Supports: Ubuntu 22.04+, Debian 12+, existing NixOS
# Tested on: Hetzner Cloud, DigitalOcean, bare metal
# =============================================================================

set -euo pipefail

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
# Pre-flight checks
# ---------------------------------------------------------------------------
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

    # nixos-infect handles everything
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

    curl -fsSL https://raw.githubusercontent.com/elitak/nixos-infect/master/nixos-infect \
      | NIX_CHANNEL=nixos-unstable PROVIDER="$PROVIDER" bash -x

    # If we get here, infect didn't reboot (shouldn't happen)
    warn "nixos-infect complete. Please reboot and re-run this script with --skip-nixos"
    exit 0
  else
    warn "NixOS installation not supported for $OS_TYPE."
    warn "Please install NixOS manually, then re-run with --skip-nixos"
    exit 1
  fi
fi

# ---------------------------------------------------------------------------
# Step 2: Install dependencies
# ---------------------------------------------------------------------------
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
  for pkg in gcc gnumake pkg-config sqlite openssl cmake; do
    if ! nix-env -q "$pkg" &>/dev/null; then
      nix-env -iA "nixos.$pkg" 2>/dev/null || true
    fi
  done
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
  fi
fi

log "Dependencies ready."

# ---------------------------------------------------------------------------
# Step 3: Clone/update the repo
# ---------------------------------------------------------------------------
log "Step 3: Getting osModa source..."

if [ -d "$INSTALL_DIR/.git" ]; then
  log "Updating existing installation..."
  cd "$INSTALL_DIR"
  git fetch origin "$BRANCH"
  git reset --hard "origin/$BRANCH"
elif [ -d "$INSTALL_DIR" ]; then
  log "Removing stale installation at $INSTALL_DIR..."
  rm -rf "$INSTALL_DIR"
  git clone --branch "$BRANCH" --depth 1 "$REPO_URL" "$INSTALL_DIR"
  cd "$INSTALL_DIR"
else
  log "Cloning osModa..."
  git clone --branch "$BRANCH" --depth 1 "$REPO_URL" "$INSTALL_DIR"
  cd "$INSTALL_DIR"
fi

log "Source ready at $INSTALL_DIR"

# ---------------------------------------------------------------------------
# Step 4: Build Rust binaries
# ---------------------------------------------------------------------------
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
for binary in agentd agentctl osmoda-egress osmoda-keyd osmoda-watch osmoda-routines osmoda-voice osmoda-mesh osmoda-mcpd osmoda-teachd; do
  if [ -f "target/release/$binary" ]; then
    ln -sf "$INSTALL_DIR/target/release/$binary" "$INSTALL_DIR/bin/$binary"
    log "Built: $binary"
  fi
done

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
log "Step 6: Setting up osmoda-bridge plugin..."

PLUGIN_SRC="$INSTALL_DIR/packages/osmoda-bridge"
PLUGIN_DST="/root/.openclaw/extensions/osmoda-bridge"

# Copy plugin to OpenClaw extensions (chown root — OpenClaw blocks non-root plugins)
mkdir -p /root/.openclaw/extensions
rm -rf "$PLUGIN_DST"
cp -r "$PLUGIN_SRC" "$PLUGIN_DST"
chown -R root:root "$PLUGIN_DST"

# Configure OpenClaw
mkdir -p /root/.openclaw
openclaw config set gateway.mode local 2>/dev/null || true
openclaw config set gateway.auth.mode none 2>/dev/null || true
openclaw config set plugins.allow '["osmoda-bridge", "device-pair", "memory-core", "phone-control", "talk-voice"]' 2>/dev/null || true

log "Bridge plugin installed with 66 system tools."

# ---------------------------------------------------------------------------
# Step 7: Install workspace templates + skills
# ---------------------------------------------------------------------------
log "Step 7: Installing agent identity and skills..."

# OpenClaw's actual workspace is ~/.openclaw/workspace/ (NOT /root/workspace/)
# Copy to both locations for compatibility
OC_WORKSPACE="/root/.openclaw/workspace"
mkdir -p "$WORKSPACE_DIR" "$OC_WORKSPACE"

# Templates — copy to both workspace dirs
for tpl in AGENTS.md SOUL.md TOOLS.md IDENTITY.md USER.md HEARTBEAT.md; do
  if [ -f "$INSTALL_DIR/templates/$tpl" ]; then
    cp "$INSTALL_DIR/templates/$tpl" "$WORKSPACE_DIR/$tpl"
    cp "$INSTALL_DIR/templates/$tpl" "$OC_WORKSPACE/$tpl"
  fi
done

# Skills — copy to both workspace dirs
if [ -d "$INSTALL_DIR/skills" ]; then
  mkdir -p "$WORKSPACE_DIR/skills" "$OC_WORKSPACE/skills"
  cp -r "$INSTALL_DIR/skills/"* "$WORKSPACE_DIR/skills/" 2>/dev/null || true
  cp -r "$INSTALL_DIR/skills/"* "$OC_WORKSPACE/skills/" 2>/dev/null || true
fi

# Create state directories with secure permissions
mkdir -p "$STATE_DIR"/{memory,ledger,config,keyd/keys,watch,routines,mesh,mcp,teachd}
mkdir -p "$RUN_DIR"
mkdir -p /var/backups/osmoda
chmod 700 "$STATE_DIR/config"
chmod 700 "$STATE_DIR/keyd"
chmod 700 "$STATE_DIR/keyd/keys"
chmod 700 "$STATE_DIR/mesh"

log "Agent identity and skills installed."

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
if [ -n "$API_KEY" ]; then
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

  # Write OpenClaw auth-profiles.json
  mkdir -p /root/.openclaw/agents/main/agent
  if command -v node &>/dev/null; then
    if [ "$EFFECTIVE_PROVIDER" = "openai" ]; then
      node -e "
        const fs = require('fs');
        const key = process.argv[1];
        const auth = { type: 'api_key', provider: 'openai', key: key };
        fs.writeFileSync('/root/.openclaw/agents/main/agent/auth-profiles.json', JSON.stringify(auth, null, 2));
      " "$DECODED_KEY"
    else
      node -e "
        const fs = require('fs');
        const key = process.argv[1];
        const isOAuth = key.startsWith('sk-ant-oat');
        const auth = isOAuth
          ? { type: 'token', provider: 'anthropic', token: key }
          : { type: 'api_key', provider: 'anthropic', key: key };
        fs.writeFileSync('/root/.openclaw/agents/main/agent/auth-profiles.json', JSON.stringify(auth, null, 2));
      " "$DECODED_KEY"
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

  log "API key configured."
else
  log "Step 8: No API key provided — setup wizard will run on port 18789."
  info "After install, open http://localhost:18789 to enter your API key."
fi

# ---------------------------------------------------------------------------
# Step 9: Create and start systemd services
# ---------------------------------------------------------------------------
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
set -uo pipefail
STATE_DIR="/var/lib/osmoda"
RUN_DIR="/run/osmoda"

OID=$(cat "$STATE_DIR/config/order-id" 2>/dev/null) || exit 0
CBURL=$(cat "$STATE_DIR/config/callback-url" 2>/dev/null) || exit 0
HBSECRET=$(cat "$STATE_DIR/config/heartbeat-secret" 2>/dev/null || echo "")

# Collect health from agentd
HEALTH=$(curl -sf --unix-socket "$RUN_DIR/agentd.sock" http://l/health 2>/dev/null || echo "{}")
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

# Collect mesh identity + peers
MESH_IDENTITY=$(curl -sf --unix-socket "$RUN_DIR/mesh.sock" http://l/identity 2>/dev/null || echo "{}")
MESH_PEERS=$(curl -sf --unix-socket "$RUN_DIR/mesh.sock" http://l/peers 2>/dev/null || echo "[]")
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
  RESPONSE=$(curl -sf -X POST "$CBURL" \
    -H "Content-Type: application/json" \
    -H "X-Heartbeat-Signature: $SIGNATURE" \
    -H "X-Heartbeat-Timestamp: $HB_TS" \
    -d "$PAYLOAD" 2>/dev/null) || exit 0
else
  RESPONSE=$(curl -sf -X POST "$CBURL" \
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
          echo "$AKEY" >> /root/.ssh/authorized_keys
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
    update_api_key)
      APROVIDER=$(echo "$ACTION_JSON" | jq -r '.provider' 2>/dev/null)
      AKEY=$(echo "$ACTION_JSON" | jq -r '.key' 2>/dev/null) || continue
      if [ -n "$AKEY" ] && [ "$AKEY" != "null" ]; then
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
        # Update auth-profiles.json (use jq to safely encode key value)
        mkdir -p /root/.openclaw/agents/main/agent
        SAFE_PROVIDER="anthropic"
        if [ "$APROVIDER" = "openai" ]; then SAFE_PROVIDER="openai"; fi
        jq -n --arg type "api_key" --arg provider "$SAFE_PROVIDER" --arg key "$AKEY" \
          '{type: $type, provider: $provider, key: $key}' \
          > /root/.openclaw/agents/main/agent/auth-profiles.json
        # Restart gateway to pick up new key
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

# Wait for agentd socket
for i in $(seq 1 10); do
  if [ -S "$RUN_DIR/agentd.sock" ]; then break; fi
  sleep 1
done

if [ -S "$RUN_DIR/agentd.sock" ]; then
  log "agentd is running."
else
  warn "agentd socket not found yet. Check: journalctl -u osmoda-agentd"
fi

# Start all daemons
for svc in osmoda-keyd osmoda-watch osmoda-routines osmoda-mesh osmoda-mcpd osmoda-teachd; do
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
fi # end SKIP_SYSTEMD

# ---------------------------------------------------------------------------
# Done!
# ---------------------------------------------------------------------------
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

# Phone home on install completion (if spawned)
if [ -n "$ORDER_ID" ] && [ -n "$CALLBACK_URL" ]; then
  log "Phoning home to spawn.os.moda..."
  if [ -n "$HEARTBEAT_SECRET" ]; then
    HB_TS=$(date +%s000)
    HB_SIG=$(printf '%s:%s' "$ORDER_ID" "$HB_TS" | openssl dgst -sha256 -hmac "$HEARTBEAT_SECRET" 2>/dev/null | awk '{print $NF}')
    curl -sf -X POST "$CALLBACK_URL" \
      -H "Content-Type: application/json" \
      -H "X-Heartbeat-Signature: $HB_SIG" \
      -H "X-Heartbeat-Timestamp: $HB_TS" \
      -d "{\"order_id\":\"$ORDER_ID\",\"status\":\"alive\",\"setup_complete\":true}" \
      || true
  else
    curl -sf -X POST "$CALLBACK_URL" \
      -H "Content-Type: application/json" \
      -d "{\"order_id\":\"$ORDER_ID\",\"status\":\"alive\",\"setup_complete\":true}" \
      || true
  fi
fi
