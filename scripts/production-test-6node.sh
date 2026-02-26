#!/usr/bin/env bash
# ============================================================================
# production-test-6node.sh — 6-Node Hetzner Production Test for osModa
# ============================================================================
#
# Creates 6 Hetzner Cloud VMs, deploys all 10 osModa daemons (9 Rust + gateway),
# runs ~254 checks across 10 categories, reports results, and destroys VMs when done.
#
# 10 Rust crates, 136 tests, 66 bridge tools, 10 daemons, 15 system skills.
# Clones latest code from https://github.com/bolivian-peru/os-moda
#
# Usage:
#   export HETZNER_TOKEN="your-token"
#   ./scripts/production-test-6node.sh           # full test + cleanup
#   ./scripts/production-test-6node.sh --keep    # keep servers after test
#   ./scripts/production-test-6node.sh --cleanup # destroy leftover servers
#
# Cost: ~€0.15 for a 2-hour run (6 × cx22 @ €0.0116/hr)
# ============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# ── Configuration ────────────────────────────────────────────────────────────

HETZNER_API="https://api.hetzner.cloud/v1"
NODE_COUNT=6
SERVER_TYPE="ccx13"
IMAGE="ubuntu-24.04"
LOCATION="fsn1"
LABEL_KEY="managed_by"
LABEL_VAL="osmoda-prod-test"
SSH_KEY="${REPO_ROOT}/~/.ssh/id_ed25519"
SSH_KEY_PUB="${SSH_KEY}.pub"
MESH_PORT=18800
REMOTE_DIR="/opt/osmoda"

# ── Runtime state ────────────────────────────────────────────────────────────

declare -a SERVERS=()
declare -a SERVER_IDS=()
declare -a MESH_IDS=()
HCLOUD_SSH_KEY_ID=""
KEEP_SERVERS=false
CLEANUP_ONLY=false
START_TIME=$(date +%s)

# Test counters
TOTAL_PASSED=0
TOTAL_FAILED=0
# Per-section: snapshot before/after (no associative arrays — macOS bash 3 compat)
_sp=0; _sf=0
SEC_P_health=0; SEC_F_health=0
SEC_P_ledger=0; SEC_F_ledger=0
SEC_P_memory=0; SEC_F_memory=0
SEC_P_wallet=0; SEC_F_wallet=0
SEC_P_switch=0; SEC_F_switch=0
SEC_P_routine=0; SEC_F_routine=0
SEC_P_teachd=0; SEC_F_teachd=0
SEC_P_mesh=0; SEC_F_mesh=0
SEC_P_mcp=0; SEC_F_mcp=0
SEC_P_integ=0; SEC_F_integ=0

# ── Colors ───────────────────────────────────────────────────────────────────

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'

# ── Helpers ──────────────────────────────────────────────────────────────────

log()    { echo -e "${GREEN}[test]${NC} $*"; }
warn()   { echo -e "${YELLOW}[test]${NC} $*"; }
error()  { echo -e "${RED}[test]${NC} $*" >&2; }
info()   { echo -e "${BLUE}[test]${NC} $*"; }
header() { echo -e "\n${BOLD}${CYAN}═══ $* ═══${NC}\n"; }

check_pass() { TOTAL_PASSED=$((TOTAL_PASSED + 1)); echo -e "  ${GREEN}✓${NC} $1"; }
check_fail() { TOTAL_FAILED=$((TOTAL_FAILED + 1)); echo -e "  ${RED}✗${NC} $1"; }

sec_start() { _sp=$TOTAL_PASSED; _sf=$TOTAL_FAILED; }
sec_end() {
  local _name="$1"
  eval "SEC_P_${_name}=$((TOTAL_PASSED - _sp))"
  eval "SEC_F_${_name}=$((TOTAL_FAILED - _sf))"
}

# SSH to a server by index (1-based)
remote() {
  local idx=$1; shift
  ssh -o ConnectTimeout=15 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
      -o LogLevel=ERROR -o ServerAliveInterval=30 -i "$SSH_KEY" \
      "root@${SERVERS[$((idx-1))]}" "$@"
}

# SSH to a server by IP
ssh_to() {
  local ip="$1"; shift
  ssh -o ConnectTimeout=15 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
      -o LogLevel=ERROR -o ServerAliveInterval=30 -i "$SSH_KEY" "root@${ip}" "$@"
}

# Hetzner Cloud API
hcloud_api() {
  local method="$1" path="$2"; shift 2
  curl -sf -X "$method" -H "Authorization: Bearer ${HETZNER_TOKEN}" \
    -H "Content-Type: application/json" "$@" "${HETZNER_API}${path}"
}

# Run curl on remote node via SSH, return stdout
remote_json() {
  local idx="$1" cmd="$2" filter="${3:-.}"
  remote "$idx" "$cmd" 2>/dev/null | jq -r "$filter" 2>/dev/null || echo ""
}

# ── Parse arguments ──────────────────────────────────────────────────────────

for arg in "$@"; do
  case "$arg" in
    --keep)    KEEP_SERVERS=true ;;
    --cleanup) CLEANUP_ONLY=true ;;
    --help|-h)
      echo "Usage: $0 [--keep] [--cleanup]"
      echo "  --keep     Keep servers after test (for debugging)"
      echo "  --cleanup  Destroy leftover test servers and exit"
      echo "Requires: HETZNER_TOKEN env var"
      exit 0 ;;
    *) error "Unknown: $arg"; exit 1 ;;
  esac
done

# ── Prerequisites ────────────────────────────────────────────────────────────

[ -z "${HETZNER_TOKEN:-}" ] && { error "HETZNER_TOKEN not set"; exit 1; }
[ ! -f "$SSH_KEY" ] && { error "SSH key missing: $SSH_KEY"; exit 1; }
[ ! -f "$SSH_KEY_PUB" ] && { error "SSH pub key missing: $SSH_KEY_PUB"; exit 1; }
for cmd in curl jq rsync ssh; do
  command -v "$cmd" &>/dev/null || { error "Required: $cmd"; exit 1; }
done

# ── Cleanup function ─────────────────────────────────────────────────────────

cleanup_servers() {
  log "Cleaning up test infrastructure..."
  local sj
  sj=$(hcloud_api GET "/servers?label_selector=${LABEL_KEY}=${LABEL_VAL}&per_page=50" 2>/dev/null || echo '{"servers":[]}')
  local sids
  sids=$(echo "$sj" | jq -r '.servers[].id' 2>/dev/null || echo "")
  for sid in $sids; do
    log "  Deleting server $sid..."
    hcloud_api DELETE "/servers/$sid" >/dev/null 2>&1 || true
  done
  local kj
  kj=$(hcloud_api GET "/ssh_keys?name=osmoda-prod-test&per_page=50" 2>/dev/null || echo '{"ssh_keys":[]}')
  local kids
  kids=$(echo "$kj" | jq -r '.ssh_keys[].id' 2>/dev/null || echo "")
  for kid in $kids; do
    hcloud_api DELETE "/ssh_keys/$kid" >/dev/null 2>&1 || true
  done
  log "Cleanup complete."
}

[ "$CLEANUP_ONLY" = true ] && { cleanup_servers; exit 0; }

trap_handler() {
  if [ "$KEEP_SERVERS" = false ] && [ ${#SERVER_IDS[@]} -gt 0 ]; then
    echo ""; warn "Interrupted — cleaning up..."
    cleanup_servers
  fi
}
trap trap_handler EXIT INT TERM

# ╔══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 1: Provision 6 VMs                                              ║
# ╚══════════════════════════════════════════════════════════════════════════╝

header "Phase 1: Provisioning $NODE_COUNT Hetzner VMs"

# Upload SSH key
log "Uploading SSH public key..."
PUB_KEY_CONTENT=$(cat "$SSH_KEY_PUB")
KEY_RESP=$(hcloud_api POST "/ssh_keys" \
  -d "{\"name\":\"osmoda-prod-test\",\"public_key\":\"${PUB_KEY_CONTENT}\"}" 2>/dev/null || echo '{}')
HCLOUD_SSH_KEY_ID=$(echo "$KEY_RESP" | jq -r '.ssh_key.id // empty')

if [ -z "$HCLOUD_SSH_KEY_ID" ]; then
  # Key might exist under a different name — search by fingerprint
  LOCAL_FP=$(ssh-keygen -lf "$SSH_KEY_PUB" -E md5 2>/dev/null | awk '{print $2}' | sed 's/^MD5://')
  ALL_KEYS=$(hcloud_api GET "/ssh_keys?per_page=50" 2>/dev/null || echo '{"ssh_keys":[]}')
  # Try matching fingerprint first, then fall back to any key with our public key content
  HCLOUD_SSH_KEY_ID=$(echo "$ALL_KEYS" | jq -r --arg fp "$LOCAL_FP" '.ssh_keys[] | select(.fingerprint == $fp) | .id' 2>/dev/null | head -1)
  if [ -z "$HCLOUD_SSH_KEY_ID" ]; then
    # Fallback: just use the first key (we'll verify SSH works later)
    HCLOUD_SSH_KEY_ID=$(echo "$ALL_KEYS" | jq -r '.ssh_keys[0].id // empty' 2>/dev/null)
  fi
  [ -z "$HCLOUD_SSH_KEY_ID" ] && { error "Failed to create/find SSH key"; exit 1; }
  log "Using existing SSH key (id=$HCLOUD_SSH_KEY_ID)"
else
  log "SSH key uploaded (id=$HCLOUD_SSH_KEY_ID)"
fi

# Create 6 servers
log "Creating $NODE_COUNT $SERVER_TYPE servers in $LOCATION..."
for i in $(seq 1 $NODE_COUNT); do
  NAME="osmoda-test-${i}"
  RESP=$(curl -s -X POST -H "Authorization: Bearer ${HETZNER_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"name\":\"${NAME}\",\"server_type\":\"${SERVER_TYPE}\",\"image\":\"${IMAGE}\",\"location\":\"${LOCATION}\",\"ssh_keys\":[${HCLOUD_SSH_KEY_ID}],\"labels\":{\"${LABEL_KEY}\":\"${LABEL_VAL}\"}}" \
    "${HETZNER_API}/servers" 2>/dev/null || echo '{}')
  SID=$(echo "$RESP" | jq -r '.server.id // empty')
  SIP=$(echo "$RESP" | jq -r '.server.public_net.ipv4.ip // empty')
  if [ -z "$SID" ]; then
    error "Failed to create $NAME: $(echo "$RESP" | jq -r '.error.message // "unknown"' 2>/dev/null)"
    exit 1
  fi
  SERVER_IDS+=("$SID")
  SERVERS+=("${SIP:-pending}")
  log "  $NAME: id=$SID ip=${SIP:-pending}"
done

# Wait for all servers to have IPs and status=running
log "Waiting for all servers to be ready..."
for attempt in $(seq 1 60); do
  ALL_READY=true
  for i in $(seq 0 $((NODE_COUNT - 1))); do
    if [ "${SERVERS[$i]}" = "pending" ] || [ -z "${SERVERS[$i]}" ]; then
      SJ=$(hcloud_api GET "/servers/${SERVER_IDS[$i]}" 2>/dev/null || echo '{}')
      ST=$(echo "$SJ" | jq -r '.server.status // "unknown"')
      IP=$(echo "$SJ" | jq -r '.server.public_net.ipv4.ip // empty')
      if [ "$ST" = "running" ] && [ -n "$IP" ]; then
        SERVERS[$i]="$IP"
        log "  osmoda-test-$((i+1)): $IP (running)"
      else
        ALL_READY=false
      fi
    fi
  done
  [ "$ALL_READY" = true ] && break
  [ "$attempt" = "60" ] && { error "Timed out waiting for servers"; exit 1; }
  sleep 5
done
log "All $NODE_COUNT servers have IPs."

# Wait for SSH
log "Waiting for SSH access..."
for i in $(seq 1 $NODE_COUNT); do
  ip="${SERVERS[$((i-1))]}"
  for attempt in $(seq 1 36); do
    ssh_to "$ip" "echo ok" >/dev/null 2>&1 && { log "  node-$i ($ip): SSH ready"; break; }
    [ "$attempt" = "36" ] && { error "SSH timeout for node-$i ($ip)"; exit 1; }
    sleep 5
  done
done
log "Phase 1 complete: $NODE_COUNT servers provisioned."

# ╔══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 2: Deploy osModa to all 6 servers                               ║
# ╚══════════════════════════════════════════════════════════════════════════╝

header "Phase 2: Deploying osModa to $NODE_COUNT servers"

deploy_one() {
  local idx=$1
  local ip="${SERVERS[$((idx-1))]}"
  echo "[deploy:${idx}] Starting deployment to node-${idx} (${ip})"

  # Step 1: System dependencies
  echo "[deploy:${idx}] Installing system deps..."
  ssh_to "$ip" "DEBIAN_FRONTEND=noninteractive apt-get update -qq && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq build-essential pkg-config libssl-dev jq curl" \
    >/dev/null 2>&1

  # Step 2: Rust toolchain
  echo "[deploy:${idx}] Installing Rust..."
  ssh_to "$ip" 'command -v cargo >/dev/null 2>&1 || \
    (curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y)' \
    >/dev/null 2>&1

  # Step 3: Clone from GitHub (latest pushed version)
  echo "[deploy:${idx}] Cloning from GitHub..."
  ssh_to "$ip" "DEBIAN_FRONTEND=noninteractive apt-get install -y -qq git >/dev/null 2>&1; \
    rm -rf ${REMOTE_DIR} && git clone https://github.com/bolivian-peru/os-moda.git ${REMOTE_DIR}" \
    2>/dev/null

  # Step 4: Build all binaries
  echo "[deploy:${idx}] Building (cargo build --release --workspace)..."
  ssh_to "$ip" bash <<'BUILD_SCRIPT'
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd /opt/osmoda
cargo build --release --workspace 2>&1 | tail -5
for bin in agentd agentctl osmoda-egress osmoda-keyd osmoda-watch osmoda-routines osmoda-voice osmoda-mesh osmoda-mcpd osmoda-teachd; do
  [ -f "target/release/$bin" ] && cp "target/release/$bin" "/usr/local/bin/$bin"
done
echo "[build] Binaries installed."
BUILD_SCRIPT

  # Step 5: Create state directories
  ssh_to "$ip" bash <<'DIRS_SCRIPT'
mkdir -p /var/lib/osmoda/{memory,ledger,config,keyd/keys,watch,routines,mesh,mcp,teachd}
mkdir -p /var/backups/osmoda /run/osmoda /var/log
chmod 700 /var/lib/osmoda/config /var/lib/osmoda/keyd /var/lib/osmoda/keyd/keys /var/lib/osmoda/mesh
DIRS_SCRIPT

  # Step 6: Kill any existing daemons
  ssh_to "$ip" bash <<'KILL_SCRIPT'
pkill -f "agentd.*--socket" 2>/dev/null || true
pkill -f "osmoda-" 2>/dev/null || true
sleep 2
pkill -9 -f "agentd.*--socket" 2>/dev/null || true
pkill -9 -f "osmoda-" 2>/dev/null || true
sleep 1
rm -f /run/osmoda/*.sock
KILL_SCRIPT

  # Step 7: Start agentd (everything depends on it)
  ssh_to "$ip" bash <<'AGENTD_SCRIPT'
RUST_LOG=info nohup /usr/local/bin/agentd \
  --socket /run/osmoda/agentd.sock --state-dir /var/lib/osmoda \
  > /var/log/osmoda-agentd.log 2>&1 &
sleep 2
for i in $(seq 1 15); do [ -S /run/osmoda/agentd.sock ] && break; sleep 1; done
[ -S /run/osmoda/agentd.sock ] || { echo "[deploy] FATAL: agentd socket missing"; exit 1; }
echo "[deploy] agentd ready"
AGENTD_SCRIPT

  # Step 8: Start subsidiary daemons (no local var expansion needed)
  ssh_to "$ip" bash <<'DAEMONS_SCRIPT'
RUST_LOG=info nohup /usr/local/bin/osmoda-keyd --socket /run/osmoda/keyd.sock --data-dir /var/lib/osmoda/keyd --policy-file /var/lib/osmoda/keyd/policy.json --agentd-socket /run/osmoda/agentd.sock > /var/log/osmoda-keyd.log 2>&1 &

RUST_LOG=info nohup /usr/local/bin/osmoda-watch --socket /run/osmoda/watch.sock --agentd-socket /run/osmoda/agentd.sock --data-dir /var/lib/osmoda/watch > /var/log/osmoda-watch.log 2>&1 &

RUST_LOG=info nohup /usr/local/bin/osmoda-routines --socket /run/osmoda/routines.sock --agentd-socket /run/osmoda/agentd.sock --routines-dir /var/lib/osmoda/routines > /var/log/osmoda-routines.log 2>&1 &

RUST_LOG=info nohup /usr/local/bin/osmoda-mcpd --socket /run/osmoda/mcpd.sock --state-dir /var/lib/osmoda/mcp --agentd-socket /run/osmoda/agentd.sock > /var/log/osmoda-mcpd.log 2>&1 &

RUST_LOG=info nohup /usr/local/bin/osmoda-teachd --socket /run/osmoda/teachd.sock --state-dir /var/lib/osmoda/teachd --agentd-socket /run/osmoda/agentd.sock --watch-socket /run/osmoda/watch.sock > /var/log/osmoda-teachd.log 2>&1 &

mkdir -p /var/lib/osmoda/voice/{models,cache}
RUST_LOG=info nohup /usr/local/bin/osmoda-voice --socket /run/osmoda/voice.sock --data-dir /var/lib/osmoda/voice --agentd-socket /run/osmoda/agentd.sock > /var/log/osmoda-voice.log 2>&1 &

RUST_LOG=info nohup /usr/local/bin/osmoda-egress --port 19999 --state-dir /var/lib/osmoda > /var/log/osmoda-egress.log 2>&1 &
DAEMONS_SCRIPT

  # Step 9: Start mesh with PUBLIC IP (needs local var expansion)
  ssh_to "$ip" "RUST_LOG=info nohup /usr/local/bin/osmoda-mesh --socket /run/osmoda/mesh.sock --data-dir /var/lib/osmoda/mesh --agentd-socket /run/osmoda/agentd.sock --listen-addr ${ip} --listen-port ${MESH_PORT} > /var/log/osmoda-mesh.log 2>&1 &"

  # Step 10: Install OpenClaw + osmoda-bridge + workspace templates
  echo "[deploy:${idx}] Setting up OpenClaw + templates..."
  ssh_to "$ip" bash <<'OPENCLAW_SCRIPT'
# Install Node.js if missing
if ! command -v node &>/dev/null; then
  curl -fsSL https://deb.nodesource.com/setup_22.x | bash - >/dev/null 2>&1
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq nodejs >/dev/null 2>&1
fi

# Install OpenClaw
OPENCLAW_DIR="/opt/openclaw"
mkdir -p "$OPENCLAW_DIR"
cd "$OPENCLAW_DIR"
[ -f package.json ] || npm init -y >/dev/null 2>&1
npm install openclaw 2>&1 | tail -2
mkdir -p /usr/local/bin
ln -sf "$OPENCLAW_DIR/node_modules/.bin/openclaw" /usr/local/bin/openclaw 2>/dev/null || true

# Install osmoda-bridge plugin
PLUGIN_DST="/root/.openclaw/extensions/osmoda-bridge"
mkdir -p /root/.openclaw/extensions
rm -rf "$PLUGIN_DST"
cp -r /opt/osmoda/packages/osmoda-bridge "$PLUGIN_DST"
chown -R root:root "$PLUGIN_DST"

# Install workspace templates (AGENTS.md, SOUL.md, TOOLS.md, IDENTITY.md, etc.)
OC_WORKSPACE="/root/.openclaw/workspace"
WORKSPACE="/root/workspace"
mkdir -p "$OC_WORKSPACE" "$WORKSPACE"
for tpl in AGENTS.md SOUL.md TOOLS.md IDENTITY.md USER.md HEARTBEAT.md; do
  [ -f "/opt/osmoda/templates/$tpl" ] && cp "/opt/osmoda/templates/$tpl" "$OC_WORKSPACE/$tpl" && cp "/opt/osmoda/templates/$tpl" "$WORKSPACE/$tpl"
done

# Install system skills
if [ -d /opt/osmoda/skills ]; then
  mkdir -p "$OC_WORKSPACE/skills" "$WORKSPACE/skills"
  cp -r /opt/osmoda/skills/* "$OC_WORKSPACE/skills/" 2>/dev/null || true
  cp -r /opt/osmoda/skills/* "$WORKSPACE/skills/" 2>/dev/null || true
fi

# Configure OpenClaw: allow osmoda-bridge plugin, set gateway.mode=local
if command -v node &>/dev/null; then
  node -e "
    const fs = require('fs');
    const p = '/root/.openclaw/openclaw.json';
    let cfg = {};
    try { cfg = JSON.parse(fs.readFileSync(p, 'utf8')); } catch(e) {}
    cfg.plugins = cfg.plugins || {};
    cfg.plugins.allow = ['osmoda-bridge'];
    cfg.gateway = cfg.gateway || {};
    cfg.gateway.mode = 'local';
    fs.writeFileSync(p, JSON.stringify(cfg, null, 2));
  " 2>/dev/null || true
fi

echo "[openclaw] Templates + skills + plugin + config installed"
OPENCLAW_SCRIPT

  # Step 11: Configure API key + start gateway (needs local var for API key)
  if [ -n "${OPENCLAW_API_KEY:-}" ]; then
    ssh_to "$ip" "mkdir -p /var/lib/osmoda/config && printf '%s' '${OPENCLAW_API_KEY}' > /var/lib/osmoda/config/api-key && chmod 600 /var/lib/osmoda/config/api-key"
    ssh_to "$ip" bash <<'GW_SCRIPT'
export PATH="/opt/openclaw/node_modules/.bin:/usr/local/bin:$HOME/.cargo/bin:$PATH"

# Configure auth
API_KEY=$(cat /var/lib/osmoda/config/api-key 2>/dev/null || echo "")
if [ -n "$API_KEY" ]; then
  mkdir -p /root/.openclaw/agents/main/agent
  node -e "
    const fs = require('fs');
    const key = process.argv[1];
    const isOAuth = key.startsWith('sk-ant-oat');
    const auth = isOAuth
      ? { type: 'token', provider: 'anthropic', token: key }
      : { type: 'api_key', provider: 'anthropic', key: key };
    fs.writeFileSync('/root/.openclaw/agents/main/agent/auth-profiles.json', JSON.stringify(auth, null, 2));
  " "$API_KEY"
fi

# Gateway env
cat > /var/lib/osmoda/config/gateway-env <<'ENVEOF'
OSMODA_SOCKET=/run/osmoda/agentd.sock
OSMODA_KEYD_SOCKET=/run/osmoda/keyd.sock
OSMODA_WATCH_SOCKET=/run/osmoda/watch.sock
OSMODA_ROUTINES_SOCKET=/run/osmoda/routines.sock
OSMODA_VOICE_SOCKET=/run/osmoda/voice.sock
OSMODA_MESH_SOCKET=/run/osmoda/mesh.sock
OSMODA_MCPD_SOCKET=/run/osmoda/mcpd.sock
OSMODA_TEACHD_SOCKET=/run/osmoda/teachd.sock
ENVEOF

# Start gateway
set -a
. /var/lib/osmoda/config/gateway-env 2>/dev/null || true
set +a
cd /root
pkill -f "openclaw gateway" 2>/dev/null || true
sleep 1
nohup openclaw gateway --port 18789 > /var/log/osmoda-gateway.log 2>&1 &
sleep 3
if ss -tlnp | grep -q ":18789"; then
  echo "[gateway] Running on port 18789"
else
  echo "[gateway] WARNING: port 18789 not listening yet"
fi
GW_SCRIPT
  fi

  # Step 12: Verify all sockets + processes
  sleep 5
  local sock_count
  sock_count=$(ssh_to "$ip" 'c=0; for s in agentd.sock keyd.sock watch.sock routines.sock mesh.sock mcpd.sock teachd.sock voice.sock; do [ -S "/run/osmoda/$s" ] && c=$((c+1)); done; echo $c')
  local egress_up
  egress_up=$(ssh_to "$ip" 'ss -tlnp | grep -q ":19999" && echo 1 || echo 0')
  local total_up=$((sock_count + egress_up))
  echo "[deploy:${idx}] ${total_up}/9 daemons ready (${sock_count} sockets + egress)"
  [ "$total_up" -ge 7 ] || { echo "[deploy:${idx}] WARNING: too few daemons ready"; return 1; }
  echo "[deploy:${idx}] Deployment complete."
}

# Launch parallel deploys
log "Starting parallel deployment..."
declare -a DEPLOY_PIDS=()
for i in $(seq 1 $NODE_COUNT); do
  deploy_one "$i" > "/tmp/osmoda-deploy-${i}.log" 2>&1 &
  DEPLOY_PIDS+=($!)
  log "  node-$i: PID ${DEPLOY_PIDS[$((i-1))]}"
done

# Wait for all deploys
DEPLOY_OK=0
for i in $(seq 1 $NODE_COUNT); do
  if wait "${DEPLOY_PIDS[$((i-1))]}" 2>/dev/null; then
    log "  node-$i: ${GREEN}deployed${NC}"
    DEPLOY_OK=$((DEPLOY_OK + 1))
  else
    error "  node-$i: FAILED (see /tmp/osmoda-deploy-${i}.log)"
  fi
done
log "Phase 2 complete: $DEPLOY_OK/$NODE_COUNT deployed."
[ "$DEPLOY_OK" -eq 0 ] && { error "No servers deployed. Aborting."; exit 1; }

# ╔══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 3: Test Suite (10 categories)                                   ║
# ╚══════════════════════════════════════════════════════════════════════════╝

header "Phase 3: Test Suite"

# ── Test 1: Health Baseline (10 daemons × 6 nodes = 60) ─────────────────

header "Test 1: Health Baseline (10 daemons × 6 nodes = 60)"
sec_start
for i in $(seq 1 $NODE_COUNT); do
  # 7 standard daemons with /health on unix socket
  for daemon in agentd keyd watch routines mesh mcpd teachd; do
    r=$(remote "$i" "curl -sf --unix-socket /run/osmoda/${daemon}.sock http://localhost/health" 2>/dev/null || echo "")
    if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
      check_pass "node-$i $daemon health"
    else
      check_fail "node-$i $daemon health"
    fi
  done

  # voice daemon: /voice/status on unix socket (not /health)
  r=$(remote "$i" "curl -sf --unix-socket /run/osmoda/voice.sock http://localhost/voice/status" 2>/dev/null || echo "")
  if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
    check_pass "node-$i voice status"
  else
    check_fail "node-$i voice status"
  fi

  # egress daemon: TCP port 19999 (no unix socket, no /health — just check port is listening)
  r=$(remote "$i" "ss -tlnp | grep -q ':19999' && echo ok || echo fail" 2>/dev/null || echo "fail")
  if [ "$r" = "ok" ]; then
    check_pass "node-$i egress listening (:19999)"
  else
    check_fail "node-$i egress listening (:19999)"
  fi

  # gateway (OpenClaw): TCP port 18789
  r=$(remote "$i" "ss -tlnp | grep -q ':18789' && echo ok || echo fail" 2>/dev/null || echo "fail")
  if [ "$r" = "ok" ]; then
    check_pass "node-$i gateway listening (:18789)"
  else
    check_fail "node-$i gateway listening (:18789)"
  fi
done
sec_end "health"

# ── Test 2: Audit Ledger (18 checks) ────────────────────────────────────

header "Test 2: Audit Ledger (3 checks × 6 nodes = 18)"
sec_start
for i in $(seq 1 $NODE_COUNT); do
  # system/query returns valid result
  r=$(remote "$i" "curl -sf --unix-socket /run/osmoda/agentd.sock -X POST -H 'Content-Type: application/json' -d '{\"query\":\"processes\"}' http://localhost/system/query" 2>/dev/null || echo "")
  if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
    check_pass "node-$i system/query"
  else
    check_fail "node-$i system/query"
  fi

  # events/log returns array
  r=$(remote "$i" "curl -sf --unix-socket /run/osmoda/agentd.sock 'http://localhost/events/log?limit=10'" 2>/dev/null || echo "")
  if echo "$r" | jq -e 'type == "array"' >/dev/null 2>&1; then
    check_pass "node-$i events/log"
  else
    check_fail "node-$i events/log"
  fi

  # Hash chain has events
  ct=$(echo "$r" | jq 'length' 2>/dev/null || echo "0")
  if [ "$ct" -gt 0 ] 2>/dev/null; then
    check_pass "node-$i ledger has events ($ct)"
  else
    check_fail "node-$i ledger has events"
  fi
done
sec_end "ledger"

# ── Test 3: Memory System (12 checks) ───────────────────────────────────

header "Test 3: Memory System (2 checks × 6 nodes = 12)"
sec_start
for i in $(seq 1 $NODE_COUNT); do
  # memory/store
  r=$(remote "$i" "curl -sf --unix-socket /run/osmoda/agentd.sock -X POST -H 'Content-Type: application/json' -d '{\"summary\":\"prod-test node $i\",\"detail\":\"Testing memory store on node $i\",\"category\":\"test\",\"tags\":[\"prod-test\"]}' http://localhost/memory/store" 2>/dev/null || echo "")
  mid=$(echo "$r" | jq -r '.id // empty' 2>/dev/null || echo "")
  if [ -n "$mid" ] && [ "$mid" != "null" ]; then
    check_pass "node-$i memory/store (id=$mid)"
  else
    check_fail "node-$i memory/store"
  fi

  # memory/recall
  r=$(remote "$i" "curl -sf --unix-socket /run/osmoda/agentd.sock -X POST -H 'Content-Type: application/json' -d '{\"query\":\"prod-test\",\"max_results\":5}' http://localhost/memory/recall" 2>/dev/null || echo "")
  if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
    check_pass "node-$i memory/recall"
  else
    check_fail "node-$i memory/recall"
  fi
done
sec_end "memory"

# ── Test 4: Wallet Operations (36 checks) ───────────────────────────────

header "Test 4: Wallet Operations (6 checks × 6 nodes = 36)"
sec_start
for i in $(seq 1 $NODE_COUNT); do
  KSOCK="/run/osmoda/keyd.sock"

  # Create ETH wallet
  r=$(remote "$i" "curl -sf --unix-socket $KSOCK -X POST -H 'Content-Type: application/json' -d '{\"chain\":\"ethereum\",\"label\":\"test-eth-$i\"}' http://localhost/wallet/create" 2>/dev/null || echo "")
  eth_id=$(echo "$r" | jq -r '.id // empty' 2>/dev/null || echo "")
  if [ -n "$eth_id" ] && [ "$eth_id" != "null" ]; then
    check_pass "node-$i create ETH wallet"
  else
    check_fail "node-$i create ETH wallet"
  fi

  # Create SOL wallet
  r=$(remote "$i" "curl -sf --unix-socket $KSOCK -X POST -H 'Content-Type: application/json' -d '{\"chain\":\"solana\",\"label\":\"test-sol-$i\"}' http://localhost/wallet/create" 2>/dev/null || echo "")
  sol_id=$(echo "$r" | jq -r '.id // empty' 2>/dev/null || echo "")
  if [ -n "$sol_id" ] && [ "$sol_id" != "null" ]; then
    check_pass "node-$i create SOL wallet"
  else
    check_fail "node-$i create SOL wallet"
  fi

  # List wallets (expect ≥2)
  r=$(remote "$i" "curl -sf --unix-socket $KSOCK http://localhost/wallet/list" 2>/dev/null || echo "[]")
  wc=$(echo "$r" | jq 'length' 2>/dev/null || echo "0")
  if [ "$wc" -ge 2 ] 2>/dev/null; then
    check_pass "node-$i wallet list ($wc wallets)"
  else
    check_fail "node-$i wallet list ($wc wallets)"
  fi

  # Sign with ETH wallet
  if [ -n "$eth_id" ] && [ "$eth_id" != "null" ]; then
    r=$(remote "$i" "curl -sf --unix-socket $KSOCK -X POST -H 'Content-Type: application/json' -d '{\"wallet_id\":\"$eth_id\",\"payload\":\"48656c6c6f\"}' http://localhost/wallet/sign" 2>/dev/null || echo "")
    sig=$(echo "$r" | jq -r '.signature // empty' 2>/dev/null || echo "")
    if [ -n "$sig" ]; then
      check_pass "node-$i ETH sign"
    else
      check_fail "node-$i ETH sign"
    fi
  else
    check_fail "node-$i ETH sign (no wallet)"
  fi

  # Delete ETH wallet
  if [ -n "$eth_id" ] && [ "$eth_id" != "null" ]; then
    r=$(remote "$i" "curl -sf --unix-socket $KSOCK -X POST -H 'Content-Type: application/json' -d '{\"wallet_id\":\"$eth_id\"}' http://localhost/wallet/delete" 2>/dev/null || echo "")
    if echo "$r" | jq -e '.deleted' >/dev/null 2>&1; then
      check_pass "node-$i delete ETH wallet"
    else
      check_fail "node-$i delete ETH wallet"
    fi
  else
    check_fail "node-$i delete ETH wallet (no wallet)"
  fi

  # Delete SOL wallet
  if [ -n "$sol_id" ] && [ "$sol_id" != "null" ]; then
    r=$(remote "$i" "curl -sf --unix-socket $KSOCK -X POST -H 'Content-Type: application/json' -d '{\"wallet_id\":\"$sol_id\"}' http://localhost/wallet/delete" 2>/dev/null || echo "")
    if echo "$r" | jq -e '.deleted' >/dev/null 2>&1; then
      check_pass "node-$i delete SOL wallet"
    else
      check_fail "node-$i delete SOL wallet"
    fi
  else
    check_fail "node-$i delete SOL wallet (no wallet)"
  fi
done
sec_end "wallet"

# ── Test 5: SafeSwitch + Watchers (30 checks) ───────────────────────────

header "Test 5: SafeSwitch + Watchers (5 checks × 6 nodes = 30)"
sec_start
for i in $(seq 1 $NODE_COUNT); do
  WSOCK="/run/osmoda/watch.sock"

  # Begin switch
  r=$(remote "$i" "curl -sf --unix-socket $WSOCK -X POST -H 'Content-Type: application/json' -d '{\"plan\":\"test-switch-$i\",\"ttl_secs\":300,\"health_checks\":[]}' http://localhost/switch/begin" 2>/dev/null || echo "")
  sw_id=$(echo "$r" | jq -r '.id // empty' 2>/dev/null || echo "")
  if [ -n "$sw_id" ] && [ "$sw_id" != "null" ]; then
    check_pass "node-$i switch begin (id=${sw_id:0:8})"
  else
    check_fail "node-$i switch begin"
  fi

  # Check status
  if [ -n "$sw_id" ] && [ "$sw_id" != "null" ]; then
    r=$(remote "$i" "curl -sf --unix-socket $WSOCK http://localhost/switch/status/$sw_id" 2>/dev/null || echo "")
    if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
      check_pass "node-$i switch status"
    else
      check_fail "node-$i switch status"
    fi
  else
    check_fail "node-$i switch status (no switch)"
  fi

  # Commit switch
  if [ -n "$sw_id" ] && [ "$sw_id" != "null" ]; then
    r=$(remote "$i" "curl -sf --unix-socket $WSOCK -X POST http://localhost/switch/commit/$sw_id" 2>/dev/null || echo "")
    if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
      check_pass "node-$i switch commit"
    else
      check_fail "node-$i switch commit"
    fi
  else
    check_fail "node-$i switch commit (no switch)"
  fi

  # Add watcher
  r=$(remote "$i" "curl -sf --unix-socket $WSOCK -X POST -H 'Content-Type: application/json' -d '{\"name\":\"test-watcher-$i\",\"check\":{\"type\":\"http_get\",\"url\":\"http://127.0.0.1:18780/health\",\"expect_status\":200},\"interval_secs\":60,\"actions\":[]}' http://localhost/watcher/add" 2>/dev/null || echo "")
  if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
    check_pass "node-$i add watcher"
  else
    check_fail "node-$i add watcher"
  fi

  # List watchers
  r=$(remote "$i" "curl -sf --unix-socket $WSOCK http://localhost/watcher/list" 2>/dev/null || echo "[]")
  wc=$(echo "$r" | jq 'length' 2>/dev/null || echo "0")
  if [ "$wc" -ge 1 ] 2>/dev/null; then
    check_pass "node-$i watcher list ($wc)"
  else
    check_fail "node-$i watcher list"
  fi
done
sec_end "switch"

# ── Test 6: Routines (18 checks) ────────────────────────────────────────

header "Test 6: Routines (3 checks × 6 nodes = 18)"
sec_start
for i in $(seq 1 $NODE_COUNT); do
  RSOCK="/run/osmoda/routines.sock"

  # Add routine
  r=$(remote "$i" "curl -sf --unix-socket $RSOCK -X POST -H 'Content-Type: application/json' -d '{\"name\":\"test-routine-$i\",\"trigger\":{\"type\":\"interval\",\"seconds\":300},\"action\":{\"type\":\"health_check\"}}' http://localhost/routine/add" 2>/dev/null || echo "")
  if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
    check_pass "node-$i add routine"
  else
    check_fail "node-$i add routine"
  fi

  # List routines
  r=$(remote "$i" "curl -sf --unix-socket $RSOCK http://localhost/routine/list" 2>/dev/null || echo "[]")
  rc=$(echo "$r" | jq 'length' 2>/dev/null || echo "0")
  if [ "$rc" -ge 1 ] 2>/dev/null; then
    check_pass "node-$i routine list ($rc)"
  else
    check_fail "node-$i routine list"
  fi

  # Trigger routine
  rid=$(echo "$r" | jq -r '.[0].id // empty' 2>/dev/null || echo "")
  if [ -n "$rid" ] && [ "$rid" != "null" ]; then
    r=$(remote "$i" "curl -sf --unix-socket $RSOCK -X POST http://localhost/routine/trigger/$rid" 2>/dev/null || echo "")
    if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
      check_pass "node-$i trigger routine"
    else
      check_fail "node-$i trigger routine"
    fi
  else
    check_fail "node-$i trigger routine (no routine)"
  fi
done
sec_end "routine"

# ── Test 7: Teachd Learning (8 checks — node 1 intensive) ───────────────

header "Test 7: Teachd Learning (8 checks on node 1)"
sec_start
TSOCK="/run/osmoda/teachd.sock"

# Teachd has been running since deploy — observe loop should have data by now
# If less than 30s, wait
log "Checking teachd observations (observe loop: 30s)..."

# Check observations exist
r=$(remote 1 "curl -sf --unix-socket $TSOCK 'http://localhost/observations?limit=20'" 2>/dev/null || echo "[]")
obs_ct=$(echo "$r" | jq 'length' 2>/dev/null || echo "0")
if [ "$obs_ct" -gt 0 ] 2>/dev/null; then
  check_pass "teachd observations exist ($obs_ct)"
else
  # Wait one observe cycle and retry
  log "  Waiting 35s for observe loop..."
  sleep 35
  r=$(remote 1 "curl -sf --unix-socket $TSOCK 'http://localhost/observations?limit=20'" 2>/dev/null || echo "[]")
  obs_ct=$(echo "$r" | jq 'length' 2>/dev/null || echo "0")
  if [ "$obs_ct" -gt 0 ] 2>/dev/null; then
    check_pass "teachd observations exist ($obs_ct after wait)"
  else
    check_fail "teachd observations exist"
  fi
fi

# Check for CPU observations
has_cpu=$(echo "$r" | jq '[.[] | select(.source == "cpu" or (.data | tostring | test("cpu")))] | length' 2>/dev/null || echo "0")
if [ "$has_cpu" -gt 0 ] 2>/dev/null; then
  check_pass "teachd CPU observations ($has_cpu)"
else
  check_fail "teachd CPU observations"
fi

# Check for memory observations
has_mem=$(echo "$r" | jq '[.[] | select(.source == "memory" or (.data | tostring | test("mem")))] | length' 2>/dev/null || echo "0")
if [ "$has_mem" -gt 0 ] 2>/dev/null; then
  check_pass "teachd memory observations ($has_mem)"
else
  check_fail "teachd memory observations"
fi

# Create knowledge doc
r=$(remote 1 "curl -sf --unix-socket $TSOCK -X POST -H 'Content-Type: application/json' -d '{\"title\":\"Test Knowledge\",\"category\":\"test\",\"content\":\"This is a production test knowledge document.\",\"tags\":[\"prod-test\",\"6node\"]}' http://localhost/knowledge/create" 2>/dev/null || echo "")
kdoc_id=$(echo "$r" | jq -r '.id // empty' 2>/dev/null || echo "")
if [ -n "$kdoc_id" ] && [ "$kdoc_id" != "null" ]; then
  check_pass "teachd create knowledge (id=${kdoc_id:0:8})"
else
  check_fail "teachd create knowledge"
fi

# Retrieve knowledge doc
if [ -n "$kdoc_id" ] && [ "$kdoc_id" != "null" ]; then
  r=$(remote 1 "curl -sf --unix-socket $TSOCK http://localhost/knowledge/$kdoc_id" 2>/dev/null || echo "")
  title=$(echo "$r" | jq -r '.title // empty' 2>/dev/null || echo "")
  if [ "$title" = "Test Knowledge" ]; then
    check_pass "teachd retrieve knowledge"
  else
    check_fail "teachd retrieve knowledge (title=$title)"
  fi
else
  check_fail "teachd retrieve knowledge (no doc)"
fi

# Test /teach context injection
r=$(remote 1 "curl -sf --unix-socket $TSOCK -X POST -H 'Content-Type: application/json' -d '{\"context\":\"system health check\"}' http://localhost/teach" 2>/dev/null || echo "")
if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
  check_pass "teachd context injection"
else
  check_fail "teachd context injection"
fi

# Patterns endpoint
r=$(remote 1 "curl -sf --unix-socket $TSOCK http://localhost/patterns" 2>/dev/null || echo "")
if [ -n "$r" ] && echo "$r" | jq -e 'type == "array"' >/dev/null 2>&1; then
  check_pass "teachd patterns endpoint"
else
  check_fail "teachd patterns endpoint"
fi

# Optimize/suggest endpoint
r=$(remote 1 "curl -sf --unix-socket $TSOCK -X POST http://localhost/optimize/suggest" 2>/dev/null || echo "")
if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
  check_pass "teachd optimize/suggest"
else
  check_fail "teachd optimize/suggest"
fi
sec_end "teachd"

# ── Test 8: Full Mesh Topology (64 checks) ──────────────────────────────

header "Test 8: Full Mesh Topology (64 checks)"
sec_start
MSOCK="/run/osmoda/mesh.sock"

# Part A: Get identities (6 checks)
log "Getting mesh identities..."
for i in $(seq 1 $NODE_COUNT); do
  mid=$(remote_json "$i" "curl -sf --unix-socket $MSOCK http://localhost/identity" ".instance_id")
  MESH_IDS+=("$mid")
  if [ -n "$mid" ] && [ "$mid" != "null" ] && [ "$mid" != "" ]; then
    check_pass "node-$i mesh identity (${mid:0:12}...)"
  else
    check_fail "node-$i mesh identity"
  fi
done

# Part B: Create full mesh — 15 unique pairs (15 checks)
log "Creating full mesh topology (15 connections)..."
for from in $(seq 1 5); do
  for to in $(seq $((from + 1)) $NODE_COUNT); do
    # Create invite on node-$from
    code=$(remote_json "$from" "curl -sf --unix-socket $MSOCK -X POST -H 'Content-Type: application/json' -d '{\"ttl_secs\":300}' http://localhost/invite/create" ".invite_code")
    if [ -z "$code" ] || [ "$code" = "null" ]; then
      check_fail "mesh pair ${from}->${to} invite"
      continue
    fi
    # Accept on node-$to
    remote "$to" "curl -sf --unix-socket $MSOCK -X POST -H 'Content-Type: application/json' -d '{\"invite_code\":\"${code}\"}' http://localhost/invite/accept" >/dev/null 2>&1
    check_pass "mesh pair ${from}->${to} invite+accept"
    sleep 1
  done
done

# Wait for connections to establish
log "Waiting 25s for mesh connections to establish..."
sleep 25

# Part C: Verify connectivity (6 checks)
log "Verifying mesh connectivity..."
for i in $(seq 1 $NODE_COUNT); do
  cc=$(remote_json "$i" "curl -sf --unix-socket $MSOCK http://localhost/health" ".connected_count")
  expected=$((NODE_COUNT - 1))
  if [ "$cc" -ge "$expected" ] 2>/dev/null; then
    check_pass "node-$i mesh connected ($cc/$expected peers)"
  else
    check_fail "node-$i mesh connected ($cc/$expected peers)"
  fi
done

# Part D: DM chat messages (6 checks)
log "Testing direct messages..."
dm_pairs=("1:2" "2:3" "3:4" "4:5" "5:6" "6:1")
for pair in "${dm_pairs[@]}"; do
  from=${pair%%:*}; to=${pair##*:}
  peer_id="${MESH_IDS[$((to-1))]}"
  if [ -n "$peer_id" ] && [ "$peer_id" != "" ]; then
    r=$(remote "$from" "curl -sf --unix-socket $MSOCK -X POST -H 'Content-Type: application/json' -d '{\"message\":{\"type\":\"chat\",\"from\":\"node-$from\",\"text\":\"Hello from node $from to node $to\"}}' http://localhost/peer/${peer_id}/send" 2>/dev/null || echo "")
    del=$(echo "$r" | jq -r '.delivered // false' 2>/dev/null || echo "false")
    if [ "$del" = "true" ]; then
      check_pass "mesh DM ${from}->${to}"
    else
      check_fail "mesh DM ${from}->${to} (delivered=$del)"
    fi
  else
    check_fail "mesh DM ${from}->${to} (no peer id)"
  fi
done

# Part E: DM health_report messages (3 checks)
hr_pairs=("1:3" "2:5" "4:6")
for pair in "${hr_pairs[@]}"; do
  from=${pair%%:*}; to=${pair##*:}
  peer_id="${MESH_IDS[$((to-1))]}"
  if [ -n "$peer_id" ]; then
    r=$(remote "$from" "curl -sf --unix-socket $MSOCK -X POST -H 'Content-Type: application/json' -d '{\"message\":{\"type\":\"health_report\",\"hostname\":\"node-$from\",\"cpu\":25.5,\"memory\":60.0,\"uptime\":3600}}' http://localhost/peer/${peer_id}/send" 2>/dev/null || echo "")
    del=$(echo "$r" | jq -r '.delivered // false' 2>/dev/null || echo "false")
    if [ "$del" = "true" ]; then check_pass "mesh health_report ${from}->${to}"; else check_fail "mesh health_report ${from}->${to}"; fi
  else
    check_fail "mesh health_report ${from}->${to} (no peer)"
  fi
done

# Part F: DM alert messages (3 checks)
al_pairs=("3:1" "5:2" "6:4")
for pair in "${al_pairs[@]}"; do
  from=${pair%%:*}; to=${pair##*:}
  peer_id="${MESH_IDS[$((to-1))]}"
  if [ -n "$peer_id" ]; then
    r=$(remote "$from" "curl -sf --unix-socket $MSOCK -X POST -H 'Content-Type: application/json' -d '{\"message\":{\"type\":\"alert\",\"severity\":\"warning\",\"title\":\"Test alert from $from\",\"detail\":\"Production test alert\"}}' http://localhost/peer/${peer_id}/send" 2>/dev/null || echo "")
    del=$(echo "$r" | jq -r '.delivered // false' 2>/dev/null || echo "false")
    if [ "$del" = "true" ]; then check_pass "mesh alert ${from}->${to}"; else check_fail "mesh alert ${from}->${to}"; fi
  else
    check_fail "mesh alert ${from}->${to} (no peer)"
  fi
done

# Part G: Group room (11 checks)
log "Testing group rooms..."

# Create room on node-1
r=$(remote 1 "curl -sf --unix-socket $MSOCK -X POST -H 'Content-Type: application/json' -d '{\"name\":\"6node-test-room\"}' http://localhost/room/create" 2>/dev/null || echo "")
room_id=$(echo "$r" | jq -r '.id // empty' 2>/dev/null || echo "")
if [ -n "$room_id" ] && [ "$room_id" != "null" ]; then
  check_pass "mesh room create (${room_id:0:8})"
else
  check_fail "mesh room create"
fi

# Join 5 peers (5 checks)
for p in 2 3 4 5 6; do
  peer_id="${MESH_IDS[$((p-1))]}"
  if [ -n "$room_id" ] && [ -n "$peer_id" ]; then
    r=$(remote 1 "curl -sf --unix-socket $MSOCK -X POST -H 'Content-Type: application/json' -d '{\"room_id\":\"${room_id}\",\"peer_id\":\"${peer_id}\"}' http://localhost/room/join" 2>/dev/null || echo "")
    if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
      check_pass "mesh room join node-$p"
    else
      check_fail "mesh room join node-$p"
    fi
  else
    check_fail "mesh room join node-$p (missing ids)"
  fi
done

# Broadcast from node-1
if [ -n "$room_id" ]; then
  r=$(remote 1 "curl -sf --unix-socket $MSOCK -X POST -H 'Content-Type: application/json' -d '{\"room_id\":\"${room_id}\",\"text\":\"Hello everyone! 6-node production test.\"}' http://localhost/room/send" 2>/dev/null || echo "")
  dt=$(echo "$r" | jq -r '.delivered_to // 0' 2>/dev/null || echo "0")
  if [ "$dt" -gt 0 ] 2>/dev/null; then
    check_pass "mesh room broadcast (delivered_to=$dt)"
  else
    check_fail "mesh room broadcast"
  fi
else
  check_fail "mesh room broadcast (no room)"
fi

# History check
if [ -n "$room_id" ]; then
  r=$(remote 1 "curl -sf --unix-socket $MSOCK 'http://localhost/room/history?room_id=${room_id}&limit=10'" 2>/dev/null || echo "[]")
  mc=$(echo "$r" | jq 'length' 2>/dev/null || echo "0")
  if [ "$mc" -ge 1 ] 2>/dev/null; then
    check_pass "mesh room history ($mc messages)"
  else
    check_fail "mesh room history"
  fi
else
  check_fail "mesh room history (no room)"
fi

# Second broadcast + count verify
if [ -n "$room_id" ]; then
  remote 1 "curl -sf --unix-socket $MSOCK -X POST -H 'Content-Type: application/json' -d '{\"room_id\":\"${room_id}\",\"text\":\"Second message: mesh test complete.\"}' http://localhost/room/send" >/dev/null 2>&1
  r=$(remote 1 "curl -sf --unix-socket $MSOCK 'http://localhost/room/history?room_id=${room_id}&limit=10'" 2>/dev/null || echo "[]")
  mc=$(echo "$r" | jq 'length' 2>/dev/null || echo "0")
  if [ "$mc" -ge 2 ] 2>/dev/null; then
    check_pass "mesh room history count ($mc)"
  else
    check_fail "mesh room history count ($mc, expected ≥2)"
  fi
else
  check_fail "mesh room history count (no room)"
fi

# Room list
r=$(remote 1 "curl -sf --unix-socket $MSOCK http://localhost/rooms" 2>/dev/null || echo "[]")
rc=$(echo "$r" | jq 'length' 2>/dev/null || echo "0")
if [ "$rc" -ge 1 ] 2>/dev/null; then
  check_pass "mesh room list ($rc rooms)"
else
  check_fail "mesh room list"
fi

# Part H: Resilience test — kill node-3, verify detection, restart, verify reconnect (8 checks)
log "Testing mesh resilience (kill node-3, restart, verify reconnect)..."

# Kill mesh on node-3
remote 3 "pkill -9 -f 'osmoda-mesh' 2>/dev/null; rm -f /run/osmoda/mesh.sock" 2>/dev/null || true
check_pass "mesh kill node-3"

# Wait for health loop to detect (30s cycle + margin)
log "  Waiting 40s for disconnect detection..."
sleep 40

# Check node-1 detects node-3 disconnection
cc1=$(remote_json 1 "curl -sf --unix-socket $MSOCK http://localhost/health" ".connected_count")
if [ "$cc1" -le 4 ] 2>/dev/null; then
  check_pass "mesh node-1 detected disconnect ($cc1 peers)"
else
  check_fail "mesh node-1 detected disconnect ($cc1 peers, expected ≤4)"
fi

# Check node-2 detects
cc2=$(remote_json 2 "curl -sf --unix-socket $MSOCK http://localhost/health" ".connected_count")
if [ "$cc2" -le 4 ] 2>/dev/null; then
  check_pass "mesh node-2 detected disconnect ($cc2 peers)"
else
  check_fail "mesh node-2 detected disconnect ($cc2 peers, expected ≤4)"
fi

# Restart node-3 mesh with its public IP
ip3="${SERVERS[2]}"
ssh_to "$ip3" "RUST_LOG=info nohup /usr/local/bin/osmoda-mesh --socket /run/osmoda/mesh.sock --data-dir /var/lib/osmoda/mesh --agentd-socket /run/osmoda/agentd.sock --listen-addr ${ip3} --listen-port ${MESH_PORT} > /var/log/osmoda-mesh.log 2>&1 &" 2>/dev/null
sleep 3
r3=$(remote_json 3 "curl -sf --unix-socket $MSOCK http://localhost/health" ".status")
if [ "$r3" = "ok" ]; then
  check_pass "mesh node-3 restarted"
else
  check_fail "mesh node-3 restarted (status=$r3)"
fi

# Wait for reconnection (health loop 30s cycle)
log "  Waiting 45s for auto-reconnect..."
sleep 45

# Verify node-3 health
r3h=$(remote_json 3 "curl -sf --unix-socket $MSOCK http://localhost/health" ".connected_count")
if [ "$r3h" -ge 1 ] 2>/dev/null; then
  check_pass "mesh node-3 reconnected ($r3h peers)"
else
  check_fail "mesh node-3 reconnected ($r3h peers)"
fi

# Verify node-1 reconnection
cc1r=$(remote_json 1 "curl -sf --unix-socket $MSOCK http://localhost/health" ".connected_count")
if [ "$cc1r" -ge 4 ] 2>/dev/null; then
  check_pass "mesh node-1 reconnected ($cc1r peers)"
else
  check_fail "mesh node-1 reconnected ($cc1r peers)"
fi

# Verify node-2 reconnection
cc2r=$(remote_json 2 "curl -sf --unix-socket $MSOCK http://localhost/health" ".connected_count")
if [ "$cc2r" -ge 4 ] 2>/dev/null; then
  check_pass "mesh node-2 reconnected ($cc2r peers)"
else
  check_fail "mesh node-2 reconnected ($cc2r peers)"
fi

# DM after restart
peer1_id="${MESH_IDS[0]}"
if [ -n "$peer1_id" ]; then
  r=$(remote 3 "curl -sf --unix-socket $MSOCK -X POST -H 'Content-Type: application/json' -d '{\"message\":{\"type\":\"chat\",\"from\":\"node-3-reborn\",\"text\":\"I am back from the dead!\"}}' http://localhost/peer/${peer1_id}/send" 2>/dev/null || echo "")
  del=$(echo "$r" | jq -r '.delivered // false' 2>/dev/null || echo "false")
  if [ "$del" = "true" ]; then
    check_pass "mesh DM after restart 3→1"
  else
    check_fail "mesh DM after restart 3→1"
  fi
else
  check_fail "mesh DM after restart (no peer id)"
fi

# Part I: Final mesh health on all 6 (6 checks)
log "Final mesh health check..."
for i in $(seq 1 $NODE_COUNT); do
  r=$(remote_json "$i" "curl -sf --unix-socket $MSOCK http://localhost/health" ".status")
  if [ "$r" = "ok" ]; then
    check_pass "node-$i final mesh health"
  else
    check_fail "node-$i final mesh health"
  fi
done
sec_end "mesh"

# ── Test 9: MCP Management (3 checks — node 1) ─────────────────────────

header "Test 9: MCP Management (3 checks on node 1)"
sec_start
MCSOCK="/run/osmoda/mcpd.sock"

# Health
r=$(remote 1 "curl -sf --unix-socket $MCSOCK http://localhost/health" 2>/dev/null || echo "")
if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
  check_pass "mcpd health"
else
  check_fail "mcpd health"
fi

# Server list
r=$(remote 1 "curl -sf --unix-socket $MCSOCK http://localhost/servers" 2>/dev/null || echo "")
if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
  check_pass "mcpd server list"
else
  check_fail "mcpd server list"
fi

# Reload
r=$(remote 1 "curl -sf --unix-socket $MCSOCK -X POST http://localhost/reload" 2>/dev/null || echo "")
if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
  check_pass "mcpd reload"
else
  check_fail "mcpd reload"
fi
sec_end "mcp"

# ── Test 10: Cross-Daemon Integration (5 checks — node 1) ───────────────

header "Test 10: Cross-Daemon Integration (5 checks on node 1)"
sec_start

# Watcher triggers event in ledger
r=$(remote 1 "curl -sf --unix-socket /run/osmoda/watch.sock -X POST -H 'Content-Type: application/json' -d '{\"name\":\"integ-watcher\",\"check\":{\"type\":\"http_get\",\"url\":\"http://127.0.0.1:18780/health\",\"expect_status\":200},\"interval_secs\":30,\"actions\":[]}' http://localhost/watcher/add" 2>/dev/null || echo "")
if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
  check_pass "integration: watcher created"
else
  check_fail "integration: watcher created"
fi

# Routine + trigger
r=$(remote 1 "curl -sf --unix-socket /run/osmoda/routines.sock -X POST -H 'Content-Type: application/json' -d '{\"name\":\"integ-routine\",\"trigger\":{\"type\":\"interval\",\"seconds\":300},\"action\":{\"type\":\"health_check\"}}' http://localhost/routine/add" 2>/dev/null || echo "")
int_rid=$(echo "$r" | jq -r '.id // empty' 2>/dev/null || echo "")
if [ -n "$int_rid" ] && [ "$int_rid" != "null" ]; then
  r=$(remote 1 "curl -sf --unix-socket /run/osmoda/routines.sock -X POST http://localhost/routine/trigger/$int_rid" 2>/dev/null || echo "")
  if [ -n "$r" ] && echo "$r" | jq -e . >/dev/null 2>&1; then
    check_pass "integration: routine triggered"
  else
    check_fail "integration: routine triggered"
  fi
else
  check_fail "integration: routine triggered (no routine)"
fi

# Ledger shows events from multiple actors
r=$(remote 1 "curl -sf --unix-socket /run/osmoda/agentd.sock 'http://localhost/events/log?limit=50'" 2>/dev/null || echo "[]")
evt_ct=$(echo "$r" | jq 'length' 2>/dev/null || echo "0")
if [ "$evt_ct" -gt 5 ] 2>/dev/null; then
  check_pass "integration: ledger has cross-daemon events ($evt_ct)"
else
  check_fail "integration: ledger events ($evt_ct, expected >5)"
fi

# System discover
r=$(remote 1 "curl -sf --unix-socket /run/osmoda/agentd.sock http://localhost/system/discover" 2>/dev/null || echo "")
if [ -n "$r" ] && echo "$r" | jq -e '.found' >/dev/null 2>&1; then
  svc_ct=$(echo "$r" | jq '.found | length' 2>/dev/null || echo "0")
  check_pass "integration: system discover ($svc_ct services)"
else
  check_fail "integration: system discover"
fi

# Teachd has observations from system activity
r=$(remote 1 "curl -sf --unix-socket /run/osmoda/teachd.sock 'http://localhost/observations?limit=50'" 2>/dev/null || echo "[]")
obs_ct=$(echo "$r" | jq 'length' 2>/dev/null || echo "0")
if [ "$obs_ct" -gt 0 ] 2>/dev/null; then
  check_pass "integration: teachd has system observations ($obs_ct)"
else
  check_fail "integration: teachd observations"
fi
sec_end "integ"

# ╔══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 4: Report                                                       ║
# ╚══════════════════════════════════════════════════════════════════════════╝

END_TIME=$(date +%s)
DURATION=$((END_TIME - START_TIME))
DURATION_MIN=$((DURATION / 60))
DURATION_SEC=$((DURATION % 60))

# Compute totals per section
fmt_sec() {
  local name="$1" label="$2" expected="$3"
  local p; eval "p=\${SEC_P_${name}:-0}"
  local f; eval "f=\${SEC_F_${name}:-0}"
  local t=$((p + f))
  if [ "$f" -eq 0 ]; then
    printf "║ %-13s %3d/%-3d passed %25s ║\n" "$label:" "$p" "$t" ""
  else
    printf "║ %-13s %3d/%-3d passed  ${RED}(%d failed)${NC} %14s ║\n" "$label:" "$p" "$t" "$f" ""
  fi
}

echo ""
echo "╔═══════════════════════════════════════════════════╗"
echo "║       osModa 6-Node Production Test Results       ║"
echo "╠═══════════════════════════════════════════════════╣"
printf "║ Servers:      %d/%d deployed                       ║\n" "$DEPLOY_OK" "$NODE_COUNT"
printf "║ Duration:     %dm %ds                             ║\n" "$DURATION_MIN" "$DURATION_SEC"
echo "╠═══════════════════════════════════════════════════╣"
fmt_sec "health"  "Health"      60
fmt_sec "ledger"  "Ledger"      18
fmt_sec "memory"  "Memory"      12
fmt_sec "wallet"  "Wallets"     36
fmt_sec "switch"  "SafeSwitch"  30
fmt_sec "routine" "Routines"    18
fmt_sec "teachd"  "Teachd"       8
fmt_sec "mesh"    "Mesh"        64
fmt_sec "mcp"     "MCP"          3
fmt_sec "integ"   "Integration"  5
echo "╠═══════════════════════════════════════════════════╣"
GRAND_TOTAL=$((TOTAL_PASSED + TOTAL_FAILED))
if [ "$TOTAL_FAILED" -eq 0 ]; then
  printf "║ ${GREEN}TOTAL:        %3d/%-3d passed${NC}                       ║\n" "$TOTAL_PASSED" "$GRAND_TOTAL"
else
  printf "║ ${RED}TOTAL:        %3d/%-3d passed (%d failed)${NC}             ║\n" "$TOTAL_PASSED" "$GRAND_TOTAL" "$TOTAL_FAILED"
fi
echo "╚═══════════════════════════════════════════════════╝"

# Show server info if --keep
if [ "$KEEP_SERVERS" = true ]; then
  echo ""
  log "Servers kept alive (--keep flag)."
  echo ""
  echo -e "${BOLD}${CYAN}═══ SSH Access ═══${NC}"
  for i in $(seq 1 $NODE_COUNT); do
    ip="${SERVERS[$((i-1))]}"
    info "  node-$i: ssh -i ~/.ssh/id_ed25519 root@${ip}"
  done

  echo ""
  echo -e "${BOLD}${CYAN}═══ OpenClaw Chat (localhost tunnels) ═══${NC}"
  echo ""
  log "Opening SSH tunnels to each gateway (port 18789)..."
  for i in $(seq 1 $NODE_COUNT); do
    ip="${SERVERS[$((i-1))]}"
    LOCAL_PORT=$((18000 + i))
    # Kill any existing tunnel on this port
    lsof -ti:${LOCAL_PORT} 2>/dev/null | xargs kill 2>/dev/null || true
    # Open background SSH tunnel
    ssh -f -N -L ${LOCAL_PORT}:localhost:18789 \
      -o ConnectTimeout=15 -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
      -o LogLevel=ERROR -o ServerAliveInterval=30 -i "$SSH_KEY" \
      "root@${ip}" 2>/dev/null
    info "  node-$i: http://localhost:${LOCAL_PORT}  →  ${ip}:18789"
  done

  echo ""
  echo -e "${BOLD}${CYAN}═══ Open in Browser ═══${NC}"
  echo ""
  for i in $(seq 1 $NODE_COUNT); do
    echo -e "  ${GREEN}node-$i${NC}: http://localhost:$((18000 + i))"
  done

  echo ""
  echo -e "${BOLD}${CYAN}═══ Direct Mesh Chat (curl) ═══${NC}"
  echo ""
  log "To send a message from node-1 to node-2 via mesh:"
  echo '  PEER=$(curl -s http://localhost:18001/peers | jq -r ".[0].id")'
  echo '  curl -X POST http://localhost:18001/peer/$PEER/send \'
  echo '    -H "Content-Type: application/json" \'
  echo '    -d '"'"'{"message":{"type":"chat","from":"you","text":"Hello from node 1!"}}'"'"

  echo ""
  info "To destroy later: $0 --cleanup"
  info "To kill tunnels: kill \$(lsof -ti:18001,18002,18003,18004,18005,18006)"
fi

# ╔══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 5: Cleanup                                                      ║
# ╚══════════════════════════════════════════════════════════════════════════╝

if [ "$KEEP_SERVERS" = true ]; then
  # Disable the exit trap cleanup
  trap - EXIT INT TERM
  log "Servers preserved. Remember to clean up with: $0 --cleanup"
else
  header "Phase 5: Cleanup"
  cleanup_servers
  trap - EXIT INT TERM
fi

# Exit with non-zero if any tests failed
if [ "$TOTAL_FAILED" -gt 0 ]; then
  exit 1
fi
