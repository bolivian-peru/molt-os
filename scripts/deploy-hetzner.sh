#!/usr/bin/env bash
# deploy-hetzner.sh -- Deploy osModa to a Hetzner server
#
# Usage:
#   ./scripts/deploy-hetzner.sh <server-ip> [ssh-key-path]
#
# What it does:
#   1. Validates SSH connectivity and key
#   2. Installs NixOS via nixos-infect (if not already NixOS)
#   3. Syncs the repo to the server via rsync
#   4. Builds all daemons (agentd, keyd, watch, routines, mesh, egress, agentctl)
#   5. Installs OpenClaw and sets up the osmoda-bridge plugin (45 tools)
#   6. Installs workspace templates (AGENTS.md, SOUL.md, TOOLS.md, etc.)
#   7. Starts all daemon systemd services
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
REMOTE_DIR="/opt/osmoda"
OSMODA_STATE="/var/lib/osmoda"
OSMODA_RUN="/run/osmoda"
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

# NOTE: StrictHostKeyChecking=accept-new trusts the host key on first connect
# and rejects if it changes later. For higher security, pre-verify the host key
# via the Hetzner console and use StrictHostKeyChecking=yes.
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

# Build rsync SSH command as an array (no eval needed)
RSYNC_SSH_ARGS=("ssh" "-o" "ConnectTimeout=10" "-o" "StrictHostKeyChecking=accept-new")
if [ -n "${SSH_KEY_OPT}" ]; then
  RSYNC_SSH_ARGS+=("${SSH_KEY_OPT}")
fi

rsync -avz --delete \
  --exclude '.git' \
  --exclude 'target' \
  --exclude 'node_modules' \
  --exclude 'dist' \
  --exclude '.direnv' \
  --exclude 'result' \
  --exclude '.keys' \
  -e "${RSYNC_SSH_ARGS[*]}" \
  "${REPO_ROOT}/" \
  "${REMOTE_USER}@${SERVER_IP}:${REMOTE_DIR}/"

log "Repo synced to ${REMOTE_DIR}."

# ---------------------------------------------------------------------------
# Step 4: Build Rust binaries on server
# ---------------------------------------------------------------------------

log "Step 4: Building all daemons on server..."

ssh_cmd bash <<'REMOTE_BUILD'
set -euo pipefail

# Ensure Rust toolchain is available
if ! command -v cargo &>/dev/null; then
  echo "[deploy] Installing Rust toolchain..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
  source "$HOME/.cargo/env"
fi

export PATH="$HOME/.cargo/bin:$PATH"

cd /opt/osmoda

echo "[deploy] Running cargo build --release..."
if ! cargo build --release --workspace 2>&1; then
  echo "[deploy] ERROR: cargo build failed!"
  exit 1
fi

# Verify and install all binaries
echo "[deploy] Installing binaries..."
for bin in agentd agentctl osmoda-egress osmoda-keyd osmoda-watch osmoda-routines osmoda-voice osmoda-mesh; do
  if [ -f "target/release/$bin" ]; then
    cp "target/release/$bin" "/usr/local/bin/$bin"
    echo "[deploy] Installed: $bin"
  else
    echo "[deploy] WARNING: Binary not found: target/release/$bin (skipping)"
  fi
done

echo "[deploy] Binaries installed:"
ls -la /usr/local/bin/agentd /usr/local/bin/agentctl /usr/local/bin/osmoda-egress \
       /usr/local/bin/osmoda-keyd /usr/local/bin/osmoda-watch /usr/local/bin/osmoda-routines \
       /usr/local/bin/osmoda-voice /usr/local/bin/osmoda-mesh 2>/dev/null || true
REMOTE_BUILD

log "Rust build complete."

# ---------------------------------------------------------------------------
# Step 5: Install OpenClaw + set up osmoda-bridge plugin
# ---------------------------------------------------------------------------

log "Step 5: Setting up OpenClaw and osmoda-bridge plugin..."

ssh_cmd bash <<'REMOTE_OPENCLAW'
set -euo pipefail

# Ensure Node.js is available
if ! command -v node &>/dev/null; then
  echo "[deploy] Node.js not found, installing..."
  if command -v nix-env &>/dev/null; then
    nix-env -iA nixos.nodejs_22 2>/dev/null || nix profile install nixpkgs#nodejs_22
  fi
fi

# Install OpenClaw in a dedicated directory (avoids global npm issues on NixOS)
OPENCLAW_DIR="/opt/openclaw"
mkdir -p "$OPENCLAW_DIR"
cd "$OPENCLAW_DIR"

if [ ! -f package.json ]; then
  npm init -y >/dev/null 2>&1
fi

echo "[deploy] Installing/updating OpenClaw..."
npm install openclaw 2>&1 | tail -3

# Make openclaw available system-wide
mkdir -p /usr/local/bin
ln -sf "$OPENCLAW_DIR/node_modules/.bin/openclaw" /usr/local/bin/openclaw 2>/dev/null || true
mkdir -p /etc/profile.d
printf 'export PATH="%s/node_modules/.bin:$PATH"\n' "$OPENCLAW_DIR" > /etc/profile.d/osmoda-openclaw.sh

echo "[deploy] OpenClaw version: $(openclaw --version 2>/dev/null || echo 'installed')"

# Set up plugin directory
PLUGIN_DIR="/root/.openclaw/plugins/osmoda-bridge"
mkdir -p /root/.openclaw/plugins

# Symlink the plugin from the repo
echo "[deploy] Linking osmoda-bridge plugin..."
rm -rf "$PLUGIN_DIR"
ln -sf /opt/osmoda/packages/osmoda-bridge "$PLUGIN_DIR"

echo "[deploy] Plugin linked: $PLUGIN_DIR -> /opt/osmoda/packages/osmoda-bridge"
ls -la "$PLUGIN_DIR/" 2>/dev/null || true

# Configure API key if present on server
if [ -f /var/lib/osmoda/config/api-key ]; then
  echo "[deploy] API key already configured."
else
  echo "[deploy] No API key found at /var/lib/osmoda/config/api-key"
  echo "[deploy] Set it with: printf 'sk-ant-...' > /var/lib/osmoda/config/api-key && chmod 600 /var/lib/osmoda/config/api-key"
fi

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
cp /opt/osmoda/templates/AGENTS.md "$WORKSPACE/AGENTS.md" 2>/dev/null || true
cp /opt/osmoda/templates/SOUL.md "$WORKSPACE/SOUL.md" 2>/dev/null || true
cp /opt/osmoda/templates/TOOLS.md "$WORKSPACE/TOOLS.md" 2>/dev/null || true
cp /opt/osmoda/templates/IDENTITY.md "$WORKSPACE/IDENTITY.md" 2>/dev/null || true
cp /opt/osmoda/templates/USER.md "$WORKSPACE/USER.md" 2>/dev/null || true
cp /opt/osmoda/templates/HEARTBEAT.md "$WORKSPACE/HEARTBEAT.md" 2>/dev/null || true

# Create state directories (matching install.sh)
mkdir -p /var/lib/osmoda/{memory,ledger,config,keyd/keys,watch,routines,mesh}
mkdir -p /var/backups/osmoda
mkdir -p /run/osmoda
chmod 700 /var/lib/osmoda/config
chmod 700 /var/lib/osmoda/keyd
chmod 700 /var/lib/osmoda/keyd/keys
chmod 700 /var/lib/osmoda/mesh

echo "[deploy] Workspace templates installed to $WORKSPACE"
ls -la "$WORKSPACE/"
REMOTE_TEMPLATES

log "Templates installed."

# ---------------------------------------------------------------------------
# Step 7: Set up and start agentd systemd service
# ---------------------------------------------------------------------------

log "Step 7: Setting up systemd services for all daemons..."

ssh_cmd bash <<'REMOTE_SYSTEMD'
set -euo pipefail

# --- agentd (core daemon — everything else depends on this) ---
cat > /etc/systemd/system/osmoda-agentd.service <<'EOF'
[Unit]
Description=osModa Kernel Bridge Daemon
After=network.target
Wants=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/agentd --socket /run/osmoda/agentd.sock --state-dir /var/lib/osmoda
Restart=always
RestartSec=5
RuntimeDirectory=osmoda
StateDirectory=osmoda
Environment=RUST_LOG=info
User=root
Group=root
ExecStartPre=/bin/mkdir -p /run/osmoda
ExecStartPre=/bin/mkdir -p /var/lib/osmoda

[Install]
WantedBy=multi-user.target
EOF

# --- keyd (crypto wallet daemon — no network) ---
if [ -f /usr/local/bin/osmoda-keyd ]; then
cat > /etc/systemd/system/osmoda-keyd.service <<'EOF'
[Unit]
Description=osModa Crypto Wallet Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=/usr/local/bin/osmoda-keyd --socket /run/osmoda/keyd.sock --data-dir /var/lib/osmoda/keyd
Restart=always
RestartSec=5
Environment=RUST_LOG=info
PrivateNetwork=true
User=root
Group=root

[Install]
WantedBy=multi-user.target
EOF
fi

# --- watch (SafeSwitch + autopilot watchers) ---
if [ -f /usr/local/bin/osmoda-watch ]; then
cat > /etc/systemd/system/osmoda-watch.service <<'EOF'
[Unit]
Description=osModa SafeSwitch + Watcher Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=/usr/local/bin/osmoda-watch --socket /run/osmoda/watch.sock --data-dir /var/lib/osmoda/watch
Restart=always
RestartSec=5
Environment=RUST_LOG=info
User=root
Group=root

[Install]
WantedBy=multi-user.target
EOF
fi

# --- routines (background automation engine) ---
if [ -f /usr/local/bin/osmoda-routines ]; then
cat > /etc/systemd/system/osmoda-routines.service <<'EOF'
[Unit]
Description=osModa Routines Automation Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=/usr/local/bin/osmoda-routines --socket /run/osmoda/routines.sock
Restart=always
RestartSec=5
Environment=RUST_LOG=info
User=root
Group=root

[Install]
WantedBy=multi-user.target
EOF
fi

# --- mesh (P2P encrypted agent-to-agent communication) ---
if [ -f /usr/local/bin/osmoda-mesh ]; then
cat > /etc/systemd/system/osmoda-mesh.service <<'EOF'
[Unit]
Description=osModa Mesh P2P Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service

[Service]
Type=simple
ExecStart=/usr/local/bin/osmoda-mesh --socket /run/osmoda/mesh.sock --data-dir /var/lib/osmoda/mesh --listen-port 18800
Restart=always
RestartSec=5
Environment=RUST_LOG=info
User=root
Group=root

[Install]
WantedBy=multi-user.target
EOF
fi

# --- egress proxy ---
if [ -f /usr/local/bin/osmoda-egress ]; then
cat > /etc/systemd/system/osmoda-egress.service <<'EOF'
[Unit]
Description=osModa Egress Proxy
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/osmoda-egress
Restart=always
RestartSec=5
Environment=RUST_LOG=info
User=root
Group=root

[Install]
WantedBy=multi-user.target
EOF
fi

# Reload and enable all services
systemctl daemon-reload

for svc in osmoda-agentd osmoda-keyd osmoda-watch osmoda-routines osmoda-mesh osmoda-egress; do
  if [ -f "/etc/systemd/system/${svc}.service" ]; then
    systemctl enable "${svc}.service"
    systemctl restart "${svc}.service"
    echo "[deploy] Started: ${svc}"
  fi
done

echo ""
echo "[deploy] Service status:"
systemctl status osmoda-agentd.service --no-pager || true

# Verify agentd socket exists
sleep 2
if [ -S /run/osmoda/agentd.sock ]; then
  echo "[deploy] agentd socket is live at /run/osmoda/agentd.sock"
else
  echo "[deploy] WARNING: agentd socket not found. Check logs: journalctl -u osmoda-agentd"
fi
REMOTE_SYSTEMD

log "All daemon services configured and started."

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

echo ""
echo "============================================="
log "Deployment complete!"
echo "============================================="
info "Server:     ${SERVER_IP}"
info "Repo:       ${REMOTE_DIR}"
info "State:      ${OSMODA_STATE}"
info "Socket:     ${OSMODA_RUN}/agentd.sock"
info "Plugin:     ~/.openclaw/plugins/osmoda-bridge"
info "Workspace:  ${WORKSPACE_DIR}"
echo ""
info "Next steps:"
info "  ssh root@${SERVER_IP}"
info "  curl --unix-socket /run/osmoda/agentd.sock http://localhost/health"
info "  openclaw  # start chatting with your OS"
echo ""
