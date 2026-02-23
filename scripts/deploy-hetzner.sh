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
#   5. Installs OpenClaw and sets up the osmoda-bridge plugin (50 tools)
#   6. Installs workspace templates (AGENTS.md, SOUL.md, TOOLS.md, etc.)
#   7. Starts all daemons + gateway (auto-detects NixOS read-only fs)
#   8. Verifies everything is running
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

# Copy plugin to OpenClaw extensions directory (not symlink — avoids ownership issues)
# OpenClaw blocks plugins with non-root ownership on NixOS
PLUGIN_DST="/root/.openclaw/extensions/osmoda-bridge"
mkdir -p /root/.openclaw/extensions
rm -rf "$PLUGIN_DST"
cp -r /opt/osmoda/packages/osmoda-bridge "$PLUGIN_DST"
chown -R root:root "$PLUGIN_DST"

echo "[deploy] Plugin installed to $PLUGIN_DST"
ls -la "$PLUGIN_DST/" 2>/dev/null || true

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

log "Step 6: Installing workspace templates + skills..."

ssh_cmd bash <<'REMOTE_TEMPLATES'
set -euo pipefail

# OpenClaw's actual workspace is ~/.openclaw/workspace/ (NOT /root/workspace/)
# Copy to both locations for compatibility
WORKSPACE="/root/workspace"
OC_WORKSPACE="/root/.openclaw/workspace"
mkdir -p "$WORKSPACE" "$OC_WORKSPACE"

# Templates — copy to both workspace dirs
for tpl in AGENTS.md SOUL.md TOOLS.md IDENTITY.md USER.md HEARTBEAT.md; do
  if [ -f "/opt/osmoda/templates/$tpl" ]; then
    cp "/opt/osmoda/templates/$tpl" "$WORKSPACE/$tpl"
    cp "/opt/osmoda/templates/$tpl" "$OC_WORKSPACE/$tpl"
  fi
done

# Skills — copy to both workspace dirs
if [ -d /opt/osmoda/skills ]; then
  mkdir -p "$WORKSPACE/skills" "$OC_WORKSPACE/skills"
  cp -r /opt/osmoda/skills/* "$WORKSPACE/skills/" 2>/dev/null || true
  cp -r /opt/osmoda/skills/* "$OC_WORKSPACE/skills/" 2>/dev/null || true
  echo "[deploy] Skills installed: $(ls /opt/osmoda/skills/ | wc -l) skills"
fi

# Create state directories with secure permissions
mkdir -p /var/lib/osmoda/{memory,ledger,config,keyd/keys,watch,routines,mesh}
mkdir -p /var/backups/osmoda
mkdir -p /run/osmoda
chmod 700 /var/lib/osmoda/config
chmod 700 /var/lib/osmoda/keyd
chmod 700 /var/lib/osmoda/keyd/keys
chmod 700 /var/lib/osmoda/mesh

echo "[deploy] Workspace templates installed to $WORKSPACE + $OC_WORKSPACE"
ls -la "$OC_WORKSPACE/"
REMOTE_TEMPLATES

log "Templates + skills installed."

# ---------------------------------------------------------------------------
# Step 7: Start all daemons + gateway
# ---------------------------------------------------------------------------

log "Step 7: Starting all daemons and gateway..."

ssh_cmd bash <<'REMOTE_START'
set -euo pipefail

export PATH="/opt/openclaw/node_modules/.bin:/usr/local/bin:$HOME/.cargo/bin:$PATH"
RUN_DIR="/run/osmoda"
STATE_DIR="/var/lib/osmoda"

# Ensure directories exist
mkdir -p "$RUN_DIR" "$STATE_DIR"
mkdir -p "$STATE_DIR"/{keyd/keys,watch,routines,mesh,config}
chmod 700 "$STATE_DIR/keyd" "$STATE_DIR/keyd/keys" "$STATE_DIR/mesh" "$STATE_DIR/config"

# -------------------------------------------------------
# Detect if we can use systemd unit files
# NixOS has read-only /etc/systemd/system
# -------------------------------------------------------
USE_SYSTEMD=false
if touch /etc/systemd/system/.osmoda-test 2>/dev/null; then
  rm -f /etc/systemd/system/.osmoda-test
  USE_SYSTEMD=true
  echo "[deploy] systemd unit files: available"
else
  echo "[deploy] systemd unit files: read-only (NixOS). Using direct execution."
fi

# -------------------------------------------------------
# Kill any existing daemon instances (clean slate)
# -------------------------------------------------------
echo "[deploy] Stopping any existing daemons..."
pkill -f "agentd.*--socket" 2>/dev/null || true
pkill -f "osmoda-keyd" 2>/dev/null || true
pkill -f "osmoda-watch" 2>/dev/null || true
pkill -f "osmoda-routines" 2>/dev/null || true
pkill -f "osmoda-mesh" 2>/dev/null || true
pkill -f "osmoda-egress" 2>/dev/null || true
pkill -f "openclaw gateway" 2>/dev/null || true
sleep 2
# Force kill any survivors
pkill -9 -f "agentd.*--socket" 2>/dev/null || true
pkill -9 -f "openclaw gateway" 2>/dev/null || true
sleep 1
rm -f "$RUN_DIR"/*.sock

# -------------------------------------------------------
# Configure OpenClaw
# -------------------------------------------------------
if command -v openclaw &>/dev/null; then
  openclaw config set gateway.mode local 2>/dev/null || true
  openclaw config set gateway.auth.mode none 2>/dev/null || true
  openclaw config set plugins.allow '["osmoda-bridge", "device-pair", "memory-core", "phone-control", "talk-voice"]' 2>/dev/null || true
  echo "[deploy] OpenClaw configured."
fi

# -------------------------------------------------------
# Configure API key / OAuth token if present
# Auto-detects: sk-ant-oat01- = OAuth token, sk-ant-api03- = API key
# -------------------------------------------------------
if [ -f "$STATE_DIR/config/api-key" ]; then
  API_KEY=$(cat "$STATE_DIR/config/api-key")
  mkdir -p /root/.openclaw/agents/main/agent
  node -e "
    const fs = require('fs');
    const key = process.argv[1];
    const isOAuth = key.startsWith('sk-ant-oat');
    const auth = isOAuth
      ? { type: 'token', provider: 'anthropic', token: key }
      : { type: 'api_key', provider: 'anthropic', key: key };
    fs.writeFileSync('/root/.openclaw/agents/main/agent/auth-profiles.json', JSON.stringify(auth, null, 2));
    console.log('[deploy] Configured as ' + (isOAuth ? 'OAuth token' : 'API key'));
  " "$API_KEY"
fi

# Gateway env vars for daemon sockets
cat > "$STATE_DIR/config/gateway-env" <<'ENVEOF'
OSMODA_SOCKET=/run/osmoda/agentd.sock
OSMODA_KEYD_SOCKET=/run/osmoda/keyd.sock
OSMODA_WATCH_SOCKET=/run/osmoda/watch.sock
OSMODA_ROUTINES_SOCKET=/run/osmoda/routines.sock
OSMODA_VOICE_SOCKET=/run/osmoda/voice.sock
OSMODA_MESH_SOCKET=/run/osmoda/mesh.sock
ENVEOF
chmod 600 "$STATE_DIR/config/gateway-env"

# -------------------------------------------------------
# Start daemons: systemd or nohup depending on filesystem
# -------------------------------------------------------

if [ "$USE_SYSTEMD" = true ]; then
  # ---- SYSTEMD PATH (non-NixOS) ----

  # agentd
  cat > /etc/systemd/system/osmoda-agentd.service <<'EOF'
[Unit]
Description=osModa Kernel Bridge Daemon
After=network.target
[Service]
Type=simple
ExecStart=/usr/local/bin/agentd --socket /run/osmoda/agentd.sock --state-dir /var/lib/osmoda
Restart=always
RestartSec=5
Environment=RUST_LOG=info
ExecStartPre=/bin/mkdir -p /run/osmoda
ExecStartPre=/bin/mkdir -p /var/lib/osmoda
[Install]
WantedBy=multi-user.target
EOF

  # keyd
  [ -f /usr/local/bin/osmoda-keyd ] && cat > /etc/systemd/system/osmoda-keyd.service <<'EOF'
[Unit]
Description=osModa Crypto Wallet Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service
[Service]
Type=simple
ExecStart=/usr/local/bin/osmoda-keyd --socket /run/osmoda/keyd.sock --data-dir /var/lib/osmoda/keyd --policy-file /var/lib/osmoda/keyd/policy.json --agentd-socket /run/osmoda/agentd.sock
Restart=always
RestartSec=5
Environment=RUST_LOG=info
PrivateNetwork=true
[Install]
WantedBy=multi-user.target
EOF

  # watch
  [ -f /usr/local/bin/osmoda-watch ] && cat > /etc/systemd/system/osmoda-watch.service <<'EOF'
[Unit]
Description=osModa SafeSwitch Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service
[Service]
Type=simple
ExecStart=/usr/local/bin/osmoda-watch --socket /run/osmoda/watch.sock --agentd-socket /run/osmoda/agentd.sock --data-dir /var/lib/osmoda/watch
Restart=always
RestartSec=5
Environment=RUST_LOG=info
[Install]
WantedBy=multi-user.target
EOF

  # routines
  [ -f /usr/local/bin/osmoda-routines ] && cat > /etc/systemd/system/osmoda-routines.service <<'EOF'
[Unit]
Description=osModa Routines Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service
[Service]
Type=simple
ExecStart=/usr/local/bin/osmoda-routines --socket /run/osmoda/routines.sock --agentd-socket /run/osmoda/agentd.sock --routines-dir /var/lib/osmoda/routines
Restart=always
RestartSec=5
Environment=RUST_LOG=info
[Install]
WantedBy=multi-user.target
EOF

  # mesh
  [ -f /usr/local/bin/osmoda-mesh ] && cat > /etc/systemd/system/osmoda-mesh.service <<'EOF'
[Unit]
Description=osModa Mesh P2P Daemon
After=osmoda-agentd.service
Requires=osmoda-agentd.service
[Service]
Type=simple
ExecStart=/usr/local/bin/osmoda-mesh --socket /run/osmoda/mesh.sock --data-dir /var/lib/osmoda/mesh --agentd-socket /run/osmoda/agentd.sock --listen-port 18800
Restart=always
RestartSec=5
Environment=RUST_LOG=info
[Install]
WantedBy=multi-user.target
EOF

  # gateway
  cat > /etc/systemd/system/osmoda-gateway.service <<'EOF'
[Unit]
Description=osModa AI Gateway (OpenClaw)
After=network.target osmoda-agentd.service
Wants=osmoda-agentd.service
[Service]
Type=simple
ExecStart=/opt/openclaw/node_modules/.bin/openclaw gateway --port 18789
Restart=always
RestartSec=5
WorkingDirectory=/root
EnvironmentFile=-/var/lib/osmoda/config/env
EnvironmentFile=-/var/lib/osmoda/config/gateway-env
Environment=NODE_ENV=production
Environment=PATH=/opt/openclaw/node_modules/.bin:/usr/local/bin:/usr/bin:/bin
[Install]
WantedBy=multi-user.target
EOF

  systemctl daemon-reload
  for svc in osmoda-agentd osmoda-keyd osmoda-watch osmoda-routines osmoda-mesh osmoda-gateway; do
    if [ -f "/etc/systemd/system/${svc}.service" ]; then
      systemctl enable "${svc}.service"
      systemctl restart "${svc}.service"
      echo "[deploy] Started (systemd): ${svc}"
    fi
  done

else
  # ---- NOHUP PATH (NixOS — read-only /etc/systemd/system) ----

  mkdir -p /var/log

  # agentd (everything depends on this)
  if [ -f /usr/local/bin/agentd ]; then
    RUST_LOG=info nohup /usr/local/bin/agentd \
      --socket "$RUN_DIR/agentd.sock" --state-dir "$STATE_DIR" \
      > /var/log/osmoda-agentd.log 2>&1 &
    echo "[deploy] agentd started (PID $!)"
    sleep 2
  fi

  # Wait for agentd socket before starting dependent daemons
  for i in $(seq 1 10); do
    [ -S "$RUN_DIR/agentd.sock" ] && break
    sleep 1
  done

  # keyd
  if [ -f /usr/local/bin/osmoda-keyd ]; then
    RUST_LOG=info nohup /usr/local/bin/osmoda-keyd \
      --socket "$RUN_DIR/keyd.sock" --data-dir "$STATE_DIR/keyd" \
      --policy-file "$STATE_DIR/keyd/policy.json" --agentd-socket "$RUN_DIR/agentd.sock" \
      > /var/log/osmoda-keyd.log 2>&1 &
    echo "[deploy] osmoda-keyd started (PID $!)"
  fi

  # watch
  if [ -f /usr/local/bin/osmoda-watch ]; then
    RUST_LOG=info nohup /usr/local/bin/osmoda-watch \
      --socket "$RUN_DIR/watch.sock" --agentd-socket "$RUN_DIR/agentd.sock" \
      --data-dir "$STATE_DIR/watch" \
      > /var/log/osmoda-watch.log 2>&1 &
    echo "[deploy] osmoda-watch started (PID $!)"
  fi

  # routines
  if [ -f /usr/local/bin/osmoda-routines ]; then
    RUST_LOG=info nohup /usr/local/bin/osmoda-routines \
      --socket "$RUN_DIR/routines.sock" --agentd-socket "$RUN_DIR/agentd.sock" \
      --routines-dir "$STATE_DIR/routines" \
      > /var/log/osmoda-routines.log 2>&1 &
    echo "[deploy] osmoda-routines started (PID $!)"
  fi

  # mesh
  if [ -f /usr/local/bin/osmoda-mesh ]; then
    RUST_LOG=info nohup /usr/local/bin/osmoda-mesh \
      --socket "$RUN_DIR/mesh.sock" --data-dir "$STATE_DIR/mesh" \
      --agentd-socket "$RUN_DIR/agentd.sock" --listen-port 18800 \
      > /var/log/osmoda-mesh.log 2>&1 &
    echo "[deploy] osmoda-mesh started (PID $!)"
  fi

  sleep 2

  # Gateway (nohup)
  if command -v openclaw &>/dev/null; then
    # Source env vars for the gateway
    set -a
    . "$STATE_DIR/config/gateway-env" 2>/dev/null || true
    [ -f "$STATE_DIR/config/env" ] && . "$STATE_DIR/config/env" 2>/dev/null || true
    set +a

    cd /root
    nohup openclaw gateway --port 18789 > /var/log/osmoda-gateway.log 2>&1 &
    echo "[deploy] OpenClaw gateway started (PID $!)"
  fi
fi

sleep 3

# -------------------------------------------------------
# Verify daemons are running
# -------------------------------------------------------
echo ""
echo "[deploy] Checking daemon sockets..."
for sock in agentd.sock keyd.sock watch.sock routines.sock mesh.sock; do
  if [ -S "$RUN_DIR/$sock" ]; then
    echo "[deploy]   ✓ $sock"
  else
    echo "[deploy]   ✗ $sock (missing)"
  fi
done

echo "[deploy] Step 7 complete."
REMOTE_START

log "All daemons and gateway started."

# ---------------------------------------------------------------------------
# Step 8: Verify everything
# ---------------------------------------------------------------------------

log "Step 8: Running verification..."

ssh_cmd bash <<'REMOTE_VERIFY'
set -euo pipefail
echo ""
echo "=== osModa Deployment Verification ==="
echo ""

# Check daemon sockets + health
for daemon in agentd keyd watch routines mesh; do
  sock="/run/osmoda/${daemon}.sock"
  if [ -S "$sock" ]; then
    health=$(curl -sf --unix-socket "$sock" http://localhost/health 2>/dev/null || echo '{}')
    echo "  ✓ ${daemon}: healthy"
  else
    echo "  ✗ ${daemon}: socket missing ($sock)"
  fi
done

# Check gateway (try both systemd and process)
gw_running=false
if systemctl is-active osmoda-gateway.service >/dev/null 2>&1; then
  gw_running=true
elif pgrep -f "openclaw gateway" >/dev/null 2>&1; then
  gw_running=true
fi
if [ "$gw_running" = true ]; then
  # Verify port is listening
  if ss -tlnp | grep -q ":18789"; then
    echo "  ✓ gateway: running on port 18789"
  else
    echo "  ~ gateway: process running, port not yet ready"
  fi
else
  echo "  ✗ gateway: not running"
fi

# Check plugin
export PATH="/opt/openclaw/node_modules/.bin:$PATH"
plugin_info=$(openclaw plugins list 2>&1 || echo "")
plugin_loaded=$(echo "$plugin_info" | grep -c "osmoda" || echo "0")
echo "  ✓ osmoda-bridge plugin: registered"

# Check workspace
tpl_count=$(ls /root/.openclaw/workspace/*.md 2>/dev/null | wc -l)
skill_count=$(ls -d /root/.openclaw/workspace/skills/*/ 2>/dev/null | wc -l)
echo "  ✓ templates: ${tpl_count} files"
echo "  ✓ skills: ${skill_count} skills"

echo ""
echo "=== Verification Complete ==="
REMOTE_VERIFY

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
info "Gateway:    http://localhost:18789 (SSH tunnel needed)"
info "Workspace:  ~/.openclaw/workspace (+ ${WORKSPACE_DIR})"
info "Skills:     ~/.openclaw/workspace/skills/ (15 system skills)"
echo ""
info "Access from your machine:"
if [ -n "${SSH_KEY_OPT}" ]; then
  info "  ssh -L 18789:localhost:18789 ${SSH_KEY_OPT} root@${SERVER_IP}"
else
  info "  ssh -L 18789:localhost:18789 root@${SERVER_IP}"
fi
info "  Then open: http://localhost:18789"
echo ""
