#!/usr/bin/env bash
# deploy-hetzner.sh -- Deploy AgentOS to a Hetzner server
#
# Usage:
#   ./scripts/deploy-hetzner.sh <server-ip> [ssh-key-path]
#
# What it does:
#   1. Validates SSH connectivity and key
#   2. Installs NixOS via nixos-infect (if not already NixOS)
#   3. Syncs the repo to the server via rsync
#   4. Builds agentd + agentctl on the server
#   5. Installs OpenClaw and sets up the agentos-bridge plugin
#   6. Installs workspace templates (AGENTS.md, SOUL.md, TOOLS.md, etc.)
#   7. Starts the agentd systemd service
#
# Prerequisites:
#   - SSH access to the server (root)
#   - rsync installed locally
#   - Server has internet access (for nixos-infect + cargo)

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
REMOTE_USER="root"
REMOTE_DIR="/opt/molt-os"
AGENTOS_STATE="/var/lib/agentos"
AGENTOS_RUN="/run/agentos"
OPENCLAW_DIR="/root/.openclaw"
WORKSPACE_DIR="/root/workspace"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

log()   { echo -e "${GREEN}[deploy]${NC} $*"; }
warn()  { echo -e "${YELLOW}[deploy]${NC} $*"; }
error() { echo -e "${RED}[deploy]${NC} $*" >&2; }
info()  { echo -e "${BLUE}[deploy]${NC} $*"; }

die() {
  error "$@"
  exit 1
}

ssh_cmd() {
  ssh -o ConnectTimeout=10 \
      -o StrictHostKeyChecking=accept-new \
      ${SSH_KEY_OPT:-} \
      "${REMOTE_USER}@${SERVER_IP}" "$@"
}

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------

if [ $# -lt 1 ]; then
  echo "Usage: $0 <server-ip> [ssh-key-path]"
  echo ""
  echo "Examples:"
  echo "  $0 65.108.x.x"
  echo "  $0 65.108.x.x ~/.ssh/hetzner_ed25519"
  exit 1
fi

SERVER_IP="$1"
SSH_KEY_OPT=""

if [ $# -ge 2 ]; then
  SSH_KEY="$2"
  if [ ! -f "$SSH_KEY" ]; then
    die "SSH key not found: $SSH_KEY"
  fi
  SSH_KEY_OPT="-i $SSH_KEY"
  log "Using SSH key: $SSH_KEY"
fi

# ---------------------------------------------------------------------------
# Step 1: Validate SSH connectivity
# ---------------------------------------------------------------------------

log "Step 1: Checking SSH connectivity to ${SERVER_IP}..."

if ! ssh_cmd "echo ok" >/dev/null 2>&1; then
  die "Cannot SSH to ${REMOTE_USER}@${SERVER_IP}. Check your key and server IP."
fi

log "SSH connection verified."

# ---------------------------------------------------------------------------
# Step 2: Check if already NixOS, otherwise run nixos-infect
# ---------------------------------------------------------------------------

log "Step 2: Checking if server is running NixOS..."

IS_NIXOS=$(ssh_cmd "[ -f /etc/NIXOS ] && echo yes || echo no")

if [ "$IS_NIXOS" = "yes" ]; then
  log "Server is already running NixOS. Skipping nixos-infect."
else
  warn "Server is NOT running NixOS. Running nixos-infect..."
  warn "This will REPLACE the server's OS with NixOS."
  echo ""
  read -p "Continue with nixos-infect? (yes/no): " CONFIRM
  if [ "$CONFIRM" != "yes" ]; then
    die "Aborted. Run nixos-infect manually if needed."
  fi

  log "Running nixos-infect on ${SERVER_IP}..."
  ssh_cmd "curl -fsSL https://raw.githubusercontent.com/elitak/nixos-infect/master/nixos-infect | NIX_CHANNEL=nixos-24.11 bash -x"

  warn "nixos-infect complete. The server will reboot."
  warn "Waiting 60 seconds for reboot..."
  sleep 60

  # Wait for SSH to come back
  for i in $(seq 1 30); do
    if ssh_cmd "echo ok" >/dev/null 2>&1; then
      log "Server is back online after nixos-infect."
      break
    fi
    if [ "$i" = "30" ]; then
      die "Server did not come back after nixos-infect. Check the console."
    fi
    sleep 10
  done
fi

# ---------------------------------------------------------------------------
# Step 3: Sync repo to server via rsync
# ---------------------------------------------------------------------------

log "Step 3: Syncing repository to ${SERVER_IP}:${REMOTE_DIR}..."

ssh_cmd "mkdir -p ${REMOTE_DIR}"

RSYNC_KEY_OPT=""
if [ -n "$SSH_KEY_OPT" ]; then
  RSYNC_KEY_OPT="-e 'ssh ${SSH_KEY_OPT}'"
fi

rsync -avz --delete \
  --exclude '.git' \
  --exclude 'target' \
  --exclude 'node_modules' \
  --exclude 'dist' \
  --exclude '.direnv' \
  --exclude 'result' \
  ${RSYNC_KEY_OPT:+$(eval echo "$RSYNC_KEY_OPT")} \
  "${REPO_ROOT}/" \
  "${REMOTE_USER}@${SERVER_IP}:${REMOTE_DIR}/"

log "Repo synced to ${REMOTE_DIR}."

# ---------------------------------------------------------------------------
# Step 4: Build Rust binaries on server
# ---------------------------------------------------------------------------

log "Step 4: Building agentd and agentctl on server..."

ssh_cmd bash <<'REMOTE_BUILD'
set -euo pipefail

# Ensure Rust toolchain is available
if ! command -v cargo &>/dev/null; then
  echo "[deploy] Installing Rust toolchain..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
fi

export PATH="$HOME/.cargo/bin:$PATH"

cd /opt/molt-os

echo "[deploy] Running cargo build --release..."
cargo build --release --workspace 2>&1

echo "[deploy] Installing binaries..."
cp target/release/agentd /usr/local/bin/agentd 2>/dev/null || true
cp target/release/agentctl /usr/local/bin/agentctl 2>/dev/null || true
cp target/release/agentos-egress /usr/local/bin/agentos-egress 2>/dev/null || true

echo "[deploy] Binaries installed."
ls -la /usr/local/bin/agent* 2>/dev/null || true
REMOTE_BUILD

log "Rust build complete."

# ---------------------------------------------------------------------------
# Step 5: Install OpenClaw + set up agentos-bridge plugin
# ---------------------------------------------------------------------------

log "Step 5: Setting up OpenClaw and agentos-bridge plugin..."

ssh_cmd bash <<'REMOTE_OPENCLAW'
set -euo pipefail

# Install OpenClaw if not present
if ! command -v openclaw &>/dev/null; then
  echo "[deploy] Installing OpenClaw..."
  if command -v npm &>/dev/null; then
    npm install -g openclaw
  elif command -v nix-env &>/dev/null; then
    echo "[deploy] npm not found. Attempting nix profile install..."
    nix profile install nixpkgs#nodejs_22
    npm install -g openclaw
  else
    echo "[deploy] WARNING: Cannot install OpenClaw -- no npm or nix available."
    echo "[deploy] Install manually: npm install -g openclaw"
  fi
fi

# Set up plugin directory
PLUGIN_DIR="/root/.openclaw/plugins/agentos-bridge"
mkdir -p "$PLUGIN_DIR"

# Symlink the plugin from the repo
echo "[deploy] Linking agentos-bridge plugin..."
rm -rf "$PLUGIN_DIR"
ln -sf /opt/molt-os/packages/agentos-bridge "$PLUGIN_DIR"

echo "[deploy] Plugin linked: $PLUGIN_DIR -> /opt/molt-os/packages/agentos-bridge"
ls -la "$PLUGIN_DIR/" 2>/dev/null || true

# Ensure OpenClaw config directory exists
mkdir -p /root/.openclaw

echo "[deploy] OpenClaw plugin setup complete."
REMOTE_OPENCLAW

log "OpenClaw setup complete."

# ---------------------------------------------------------------------------
# Step 6: Install workspace templates
# ---------------------------------------------------------------------------

log "Step 6: Installing workspace templates..."

ssh_cmd bash <<'REMOTE_TEMPLATES'
set -euo pipefail

WORKSPACE="/root/workspace"
mkdir -p "$WORKSPACE"

# Copy templates
cp /opt/molt-os/templates/AGENTS.md "$WORKSPACE/AGENTS.md" 2>/dev/null || true
cp /opt/molt-os/templates/SOUL.md "$WORKSPACE/SOUL.md" 2>/dev/null || true
cp /opt/molt-os/templates/TOOLS.md "$WORKSPACE/TOOLS.md" 2>/dev/null || true
cp /opt/molt-os/templates/IDENTITY.md "$WORKSPACE/IDENTITY.md" 2>/dev/null || true
cp /opt/molt-os/templates/USER.md "$WORKSPACE/USER.md" 2>/dev/null || true
cp /opt/molt-os/templates/HEARTBEAT.md "$WORKSPACE/HEARTBEAT.md" 2>/dev/null || true

# Create state directories
mkdir -p /var/lib/agentos/memory
mkdir -p /var/lib/agentos/ledger
mkdir -p /run/agentos

echo "[deploy] Workspace templates installed to $WORKSPACE"
ls -la "$WORKSPACE/"
REMOTE_TEMPLATES

log "Templates installed."

# ---------------------------------------------------------------------------
# Step 7: Set up and start agentd systemd service
# ---------------------------------------------------------------------------

log "Step 7: Setting up agentd systemd service..."

ssh_cmd bash <<'REMOTE_SYSTEMD'
set -euo pipefail

# Create systemd service file for agentd
cat > /etc/systemd/system/agentd.service <<'EOF'
[Unit]
Description=AgentOS Kernel Bridge Daemon
After=network.target
Wants=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/agentd --socket /run/agentos/agentd.sock --state-dir /var/lib/agentos
Restart=always
RestartSec=5
RuntimeDirectory=agentos
StateDirectory=agentos
Environment=RUST_LOG=info

# agentd runs as root -- it IS the system
User=root
Group=root

# Ensure socket directory exists
ExecStartPre=/bin/mkdir -p /run/agentos
ExecStartPre=/bin/mkdir -p /var/lib/agentos

[Install]
WantedBy=multi-user.target
EOF

# Reload and start
systemctl daemon-reload
systemctl enable agentd.service
systemctl restart agentd.service

echo "[deploy] agentd service status:"
systemctl status agentd.service --no-pager || true

# Verify socket exists
sleep 2
if [ -S /run/agentos/agentd.sock ]; then
  echo "[deploy] agentd socket is live at /run/agentos/agentd.sock"
else
  echo "[deploy] WARNING: agentd socket not found. Check logs: journalctl -u agentd"
fi
REMOTE_SYSTEMD

log "agentd service configured and started."

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

echo ""
echo "============================================="
log "Deployment complete!"
echo "============================================="
info "Server:     ${SERVER_IP}"
info "Repo:       ${REMOTE_DIR}"
info "State:      ${AGENTOS_STATE}"
info "Socket:     ${AGENTOS_RUN}/agentd.sock"
info "Plugin:     ~/.openclaw/plugins/agentos-bridge"
info "Workspace:  ${WORKSPACE_DIR}"
echo ""
info "Next steps:"
info "  ssh root@${SERVER_IP}"
info "  curl --unix-socket /run/agentos/agentd.sock http://localhost/health"
info "  openclaw  # start chatting with your OS"
echo ""
