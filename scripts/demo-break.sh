#!/usr/bin/env bash
# =============================================================================
# osModa Demo Break Scripts
#
# Safe commands to break your demo stack for filming the self-healing demo.
# Each command is designed to trigger osModa's watchdog and self-healing.
#
# Usage:
#   ./scripts/demo-break.sh [command]
#
# Commands:
#   postgres-stop    - Stop PostgreSQL service
#   postgres-kill    - Kill PostgreSQL process
#   app-stop         - Stop Todo API service
#   app-kill         - Kill Todo API process
#   nginx-stop       - Stop Nginx service
#   nginx-config     - Delete Nginx config (THE MAIN DEMO)
#   nginx-kill       - Kill Nginx process
#   all              - Stop all services
#   stress           - Stress test (memory/CPU)
#
# ⚠️  WARNING: These commands will break your services!
#    Only run these when you're ready to film the recovery.
#    osModa should automatically heal after each break.
# =============================================================================

set -eo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

log()   { echo -e "${GREEN}[break]${NC} $*"; }
warn()  { echo -e "${YELLOW}[break]${NC} $*"; }
error() { echo -e "${RED}[break]${NC} $*" >&2; }
info()  { echo -e "${BLUE}[break]${NC} $*"; }

die() { error "$@"; exit 1; }

# ---------------------------------------------------------------------------
# Check if running as root
# ---------------------------------------------------------------------------
if [ "$EUID" -ne 0 ]; then
  die "Please run as root (sudo ./scripts/demo-break.sh <command>)"
fi

# ---------------------------------------------------------------------------
# Break Commands
# ---------------------------------------------------------------------------

break_postgres_stop() {
  warn "⚠️  Stopping PostgreSQL service..."
  info "This should trigger osModa watchdog to restart it."
  systemctl stop postgresql
  log "PostgreSQL stopped. Watch for self-healing!"
  info "Check status: systemctl status postgresql"
}

break_postgres_kill() {
  warn "⚠️  Killing PostgreSQL process..."
  info "This should trigger osModa watchdog to restart it."
  pkill -9 postgres || true
  log "PostgreSQL killed. Watch for self-healing!"
}

break_app_stop() {
  warn "⚠️  Stopping Todo API service..."
  info "This should trigger osModa watchdog to restart it."
  systemctl stop todo-app
  log "Todo API stopped. Watch for self-healing!"
  info "Check status: systemctl status todo-app"
}

break_app_kill() {
  warn "⚠️  Killing Todo API process..."
  info "This should trigger osModa watchdog to restart it."
  pkill -9 -f 'node.*server.js' || true
  log "Todo API killed. Watch for self-healing!"
}

break_nginx_stop() {
  warn "⚠️  Stopping Nginx service..."
  info "This should trigger osModa watchdog to restart it."
  systemctl stop nginx
  log "Nginx stopped. Watch for self-healing!"
  info "Check status: systemctl status nginx"
}

break_nginx_config() {
  warn "⚠️  ⚠️  DELETING NGINX CONFIGURATION! ⚠️  ⚠️"
  info "This is THE main demo - deleting /etc/nginx/"
  info "osModa should detect this and rollback via NixOS."
  echo ""
  echo "🎬 CAMERA READY? This is the money shot!"
  echo ""
  sleep 3
  
  # Show current nginx status
  info "Before deletion:"
  nginx -t 2>&1 || true
  echo ""
  
  # Delete the config
  info "Deleting /etc/nginx/sites-enabled/..."
  rm -rf /etc/nginx/sites-enabled/*
  info "Deleting /etc/nginx/sites-available/..."
  rm -rf /etc/nginx/sites-available/*
  
  echo ""
  log "💥 NGINX CONFIG DELETED!"
  info "Watch osModa detect and heal this!"
  info "Check nginx status: systemctl status nginx"
  info "Test: curl http://localhost/"
}

break_nginx_kill() {
  warn "⚠️  Killing Nginx process..."
  info "This should trigger osModa watchdog to restart it."
  pkill -9 nginx || true
  log "Nginx killed. Watch for self-healing!"
}

break_all() {
  warn "⚠️  ⚠️  STOPPING ALL SERVICES! ⚠️  ⚠️"
  info "This will break everything - PostgreSQL, Todo API, and Nginx."
  echo ""
  sleep 2
  
  systemctl stop postgresql
  systemctl stop todo-app
  systemctl stop nginx
  
  log "💥 All services stopped!"
  info "Watch osModa heal everything!"
}

break_stress() {
  warn "⚠️  Running stress test..."
  info "This will consume 90% of memory to trigger osModa's protection."
  
  if ! command -v stress &> /dev/null; then
    info "Installing stress tool..."
    apt-get install -y -qq stress
  fi
  
  info "Starting stress test (30 seconds)..."
  stress --vm 1 --vm-bytes 90% --timeout 30s &
  log "Stress test running. Watch osModa detect and kill it!"
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

show_help() {
  cat << EOF
${BOLD}osModa Demo Break Commands${NC}

Usage: sudo ./scripts/demo-break.sh <command>

${BOLD}Commands:${NC}
  postgres-stop    Stop PostgreSQL service (gentle)
  postgres-kill    Kill PostgreSQL process (aggressive)
  app-stop         Stop Todo API service (gentle)
  app-kill         Kill Todo API process (aggressive)
  nginx-stop       Stop Nginx service (gentle)
  nginx-config     Delete Nginx config (THE MAIN DEMO!) 🎬
  nginx-kill       Kill Nginx process (aggressive)
  all              Stop all services at once
  stress           Run memory stress test

${YELLOW}⚠️  WARNING: These commands will break your services!${NC}
   Only run when you're ready to film the recovery.
   osModa should automatically heal after each break.

${BLUE}💡 Tip: Start with 'nginx-config' for the most dramatic demo.${NC}
EOF
}

if [ $# -eq 0 ]; then
  show_help
  exit 0
fi

case $1 in
  postgres-stop)
    break_postgres_stop
    ;;
  postgres-kill)
    break_postgres_kill
    ;;
  app-stop)
    break_app_stop
    ;;
  app-kill)
    break_app_kill
    ;;
  nginx-stop)
    break_nginx_stop
    ;;
  nginx-config|nginx-delete)
    break_nginx_config
    ;;
  nginx-kill)
    break_nginx_kill
    ;;
  all)
    break_all
    ;;
  stress)
    break_stress
    ;;
  help|-h|--help)
    show_help
    ;;
  *)
    error "Unknown command: $1"
    echo ""
    show_help
    exit 1
    ;;
esac
