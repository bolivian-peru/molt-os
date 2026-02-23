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
#   4. Sets up the osmoda-bridge plugin (37 system tools)
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

while [[ $# -gt 0 ]]; do
  case $1 in
    --skip-nixos)     SKIP_NIXOS=true; shift ;;
    --api-key)        API_KEY="$2"; shift 2 ;;
    --branch)         BRANCH="$2"; shift 2 ;;
    --order-id)       ORDER_ID="$2"; shift 2 ;;
    --callback-url)   CALLBACK_URL="$2"; shift 2 ;;
    --help|-h)
      echo "osModa Installer v${VERSION}"
      echo ""
      echo "Usage: curl -fsSL <url> | bash -s -- [options]"
      echo ""
      echo "Options:"
      echo "  --skip-nixos        Don't install NixOS (use on existing NixOS systems)"
      echo "  --api-key KEY       Set Anthropic API key (skips setup wizard)"
      echo "  --branch NAME       Git branch to install (default: main)"
      echo "  --order-id UUID     Spawn order ID (set by spawn.os.moda)"
      echo "  --callback-url URL  Heartbeat callback URL (set by spawn.os.moda)"
      echo "  --help              Show this help"
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
log "Step 4: Building agentd (this takes 2-5 minutes on first build)..."

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
for binary in agentd agentctl osmoda-egress; do
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

if ! command -v openclaw &>/dev/null; then
  mkdir -p "$OPENCLAW_DIR"
  cd "$OPENCLAW_DIR"
  if [ ! -f package.json ]; then
    npm init -y >/dev/null 2>&1
  fi
  npm install openclaw 2>&1 | tail -3
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
PLUGIN_DST="$INSTALL_DIR/osmoda-bridge-plugin"

# Copy plugin files (not symlink — avoids path issues)
mkdir -p "$PLUGIN_DST"
cp "$PLUGIN_SRC/index.ts" "$PLUGIN_DST/"
cp "$PLUGIN_SRC/package.json" "$PLUGIN_DST/"
cp "$PLUGIN_SRC/openclaw.plugin.json" "$PLUGIN_DST/"

# Configure OpenClaw
mkdir -p /root/.openclaw
openclaw config set gateway.auth.mode none 2>/dev/null || true
openclaw config set plugins.allow '["osmoda-bridge"]' 2>/dev/null || true

log "Bridge plugin installed with 37 system tools."

# ---------------------------------------------------------------------------
# Step 7: Install workspace templates + skills
# ---------------------------------------------------------------------------
log "Step 7: Installing agent identity and skills..."

mkdir -p "$WORKSPACE_DIR"

# Templates
for tpl in AGENTS.md SOUL.md TOOLS.md IDENTITY.md USER.md HEARTBEAT.md; do
  if [ -f "$INSTALL_DIR/templates/$tpl" ]; then
    cp "$INSTALL_DIR/templates/$tpl" "$WORKSPACE_DIR/$tpl"
  fi
done

# Skills — copy all skill directories
if [ -d "$INSTALL_DIR/skills" ]; then
  mkdir -p "$WORKSPACE_DIR/skills"
  cp -r "$INSTALL_DIR/skills/"* "$WORKSPACE_DIR/skills/" 2>/dev/null || true
fi

# Create state directories with secure permissions
mkdir -p "$STATE_DIR"/{memory,ledger,config,keyd/keys,watch,routines}
mkdir -p "$RUN_DIR"
mkdir -p /var/backups/osmoda
chmod 700 "$STATE_DIR/config"
chmod 700 "$STATE_DIR/keyd"
chmod 700 "$STATE_DIR/keyd/keys"

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
fi

# ---------------------------------------------------------------------------
# Step 8: Set up API key (if provided) or prep setup wizard
# ---------------------------------------------------------------------------
if [ -n "$API_KEY" ]; then
  log "Step 8: Configuring API key..."
  printf '%s\n' "$API_KEY" > "$STATE_DIR/config/api-key"
  chmod 600 "$STATE_DIR/config/api-key"
  printf 'ANTHROPIC_API_KEY=%s\n' "$API_KEY" > "$STATE_DIR/config/env"
  chmod 600 "$STATE_DIR/config/env"
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
  # On NixOS, services should be managed via osmoda.nix module, not imperative unit files.
  # Start services directly if binaries exist, but don't write unit files.
  log "NixOS detected. Use the osmoda.nix NixOS module for proper service management."
  log "Starting agentd directly for now..."

  mkdir -p "$RUN_DIR" "$STATE_DIR"
  # Kill any existing agentd instance
  pkill -f "agentd.*--socket" 2>/dev/null || true
  rm -f "$RUN_DIR/agentd.sock"
  sleep 1
  if [ -f "$INSTALL_DIR/bin/agentd" ]; then
    "$INSTALL_DIR/bin/agentd" --socket "$RUN_DIR/agentd.sock" --state-dir "$STATE_DIR" &
    AGENTD_PID=$!
    log "agentd started (PID $AGENTD_PID). Add osmoda.nix module for persistent service."
  fi

  # Skip to done
  SKIP_SYSTEMD=true
else
  SKIP_SYSTEMD=false
  SYSTEMD_DIR="/etc/systemd/system"
fi

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

[Install]
WantedBy=multi-user.target
EOF

# Heartbeat timer (phones home to spawn.os.moda)
if [ -n "$ORDER_ID" ] && [ -n "$CALLBACK_URL" ]; then
cat > "$SYSTEMD_DIR/osmoda-heartbeat.service" <<EOF
[Unit]
Description=osModa Heartbeat (phones home to spawn.os.moda)
After=network-online.target osmoda-agentd.service
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=/bin/bash -c '\
  OID=\$(cat $STATE_DIR/config/order-id 2>/dev/null) && \
  CBURL=\$(cat $STATE_DIR/config/callback-url 2>/dev/null) && \
  HEALTH=\$(curl -sf --unix-socket $RUN_DIR/agentd.sock http://l/health 2>/dev/null || echo "{}") && \
  CPU=\$(echo "\$HEALTH" | grep -o "\"cpu\":[0-9.]*" | head -1 | cut -d: -f2) && \
  RAM=\$(echo "\$HEALTH" | grep -o "\"ram\":[0-9.]*" | head -1 | cut -d: -f2) && \
  DISK=\$(echo "\$HEALTH" | grep -o "\"disk\":[0-9.]*" | head -1 | cut -d: -f2) && \
  UPTIME=\$(echo "\$HEALTH" | grep -o "\"uptime\":[0-9.]*" | head -1 | cut -d: -f2) && \
  OC_READY=\$(systemctl is-active osmoda-gateway.service 2>/dev/null | grep -q "^active\$" && echo true || echo false) && \
  curl -sf -X POST "\$CBURL" \
    -H "Content-Type: application/json" \
    -d "{\"order_id\":\"\$OID\",\"status\":\"alive\",\"setup_complete\":true,\"openclaw_ready\":\$OC_READY,\"health\":{\"cpu\":\${CPU:-0},\"ram\":\${RAM:-0},\"disk\":\${DISK:-0},\"uptime\":\${UPTIME:-0}}}" \
  || true'
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
  curl -sf -X POST "$CALLBACK_URL" \
    -H "Content-Type: application/json" \
    -d "{\"order_id\":\"$ORDER_ID\",\"status\":\"alive\",\"setup_complete\":true}" \
    || true
fi
