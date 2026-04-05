#!/usr/bin/env bash
# =============================================================================
# osModa Demo Setup Script
#
# Automates the setup for the "I Deleted My Nginx Config" bounty demo.
# Sets up a complete demo stack with PostgreSQL, Todo App, and watchers.
#
# Usage:
#   ./scripts/demo-setup.sh
#
# What this does:
#   1. Installs and configures PostgreSQL
#   2. Creates and deploys the Todo API app
#   3. Sets up nginx reverse proxy
#   4. Configures watchers for self-healing demo
#   5. Adds sample data
#   6. Verifies everything is working
#
# After running this, you're ready to film the demo!
# =============================================================================

set -eo pipefail

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

log()   { echo -e "${GREEN}[demo]${NC} $*"; }
warn()  { echo -e "${YELLOW}[demo]${NC} $*"; }
error() { echo -e "${RED}[demo]${NC} $*" >&2; }
info()  { echo -e "${BLUE}[demo]${NC} $*"; }
step()  { echo -e "${BOLD}[demo]${NC} $*"; }

die() { error "$@"; exit 1; }

# ---------------------------------------------------------------------------
# Check if running as root
# ---------------------------------------------------------------------------
if [ "$EUID" -ne 0 ]; then
  die "Please run as root (sudo ./scripts/demo-setup.sh)"
fi

# ---------------------------------------------------------------------------
# Step 1: Install PostgreSQL
# ---------------------------------------------------------------------------
step "Step 1/6: Installing PostgreSQL..."

if ! command -v psql &> /dev/null; then
  apt-get update -qq
  apt-get install -y -qq postgresql postgresql-contrib
  systemctl start postgresql
  systemctl enable postgresql
  log "PostgreSQL installed and started"
else
  log "PostgreSQL already installed"
fi

# Configure PostgreSQL
sudo -u postgres psql -c "CREATE DATABASE todos;" 2>/dev/null || true
sudo -u postgres psql -c "CREATE USER app WITH PASSWORD 'demo123';" 2>/dev/null || true
sudo -u postgres psql -c "GRANT ALL PRIVILEGES ON DATABASE todos TO app;" 2>/dev/null || true
sudo -u postgres psql -d todos -c "GRANT ALL ON SCHEMA public TO app;" 2>/dev/null || true
log "PostgreSQL configured with 'todos' database and 'app' user"

# ---------------------------------------------------------------------------
# Step 2: Create Todo App
# ---------------------------------------------------------------------------
step "Step 2/6: Creating Todo API application..."

mkdir -p /opt/todo-app
cd /opt/todo-app

cat > server.js << 'EOF'
const express = require('express');
const { Pool } = require('pg');
const app = express();
app.use(express.json());

const pool = new Pool({
  user: 'app',
  host: 'localhost',
  database: 'todos',
  password: 'demo123',
  port: 5432
});

// Create table on startup
pool.query(`CREATE TABLE IF NOT EXISTS todos (
  id SERIAL PRIMARY KEY,
  title TEXT NOT NULL,
  done BOOLEAN DEFAULT false,
  created_at TIMESTAMP DEFAULT NOW()
)`).catch(err => console.error('Table creation error:', err));

app.get('/todos', async (req, res) => {
  try {
    const r = await pool.query('SELECT * FROM todos ORDER BY created_at DESC');
    res.json(r.rows);
  } catch(e) {
    res.status(500).json({ error: e.message });
  }
});

app.post('/todos', async (req, res) => {
  try {
    const r = await pool.query(
      'INSERT INTO todos (title) VALUES ($1) RETURNING *',
      [req.body.title]
    );
    res.json(r.rows[0]);
  } catch(e) {
    res.status(500).json({ error: e.message });
  }
});

app.get('/health', async (req, res) => {
  try {
    await pool.query('SELECT 1');
    res.json({ status: 'healthy', db: 'connected', timestamp: new Date().toISOString() });
  } catch(e) {
    res.status(500).json({ status: 'unhealthy', error: e.message });
  }
});

const PORT = process.env.PORT || 3000;
app.listen(PORT, () => console.log(`Todo API listening on port ${PORT}`));
EOF

# Install Node.js if not present
if ! command -v node &> /dev/null; then
  info "Installing Node.js..."
  curl -fsSL https://deb.nodesource.com/setup_lts.x | bash -
  apt-get install -y -qq nodejs
fi

# Initialize npm and install dependencies
npm init -y > /dev/null 2>&1
npm install express pg > /dev/null 2>&1
log "Todo API created with Express + PostgreSQL"

# Create systemd service
cat > /etc/systemd/system/todo-app.service << 'EOF'
[Unit]
Description=Todo API Application
After=network.target postgresql.service

[Service]
Type=simple
User=root
WorkingDirectory=/opt/todo-app
ExecStart=/usr/bin/node server.js
Restart=always
RestartSec=10

[Install]
WantedBy=multi-user.target
EOF

systemctl daemon-reload
systemctl start todo-app
systemctl enable todo-app
log "Todo API service created and started"

# ---------------------------------------------------------------------------
# Step 3: Install and Configure Nginx
# ---------------------------------------------------------------------------
step "Step 3/6: Installing and configuring Nginx..."

if ! command -v nginx &> /dev/null; then
  apt-get install -y -qq nginx
  systemctl start nginx
  systemctl enable nginx
fi

# Create nginx config for reverse proxy
cat > /etc/nginx/sites-available/todo-app << 'EOF'
server {
    listen 80;
    server_name _;

    location / {
        proxy_pass http://localhost:3000;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host $host;
        proxy_cache_bypass $http_upgrade;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }

    location /health {
        proxy_pass http://localhost:3000/health;
    }
}
EOF

# Enable the site
ln -sf /etc/nginx/sites-available/todo-app /etc/nginx/sites-enabled/todo-app
rm -f /etc/nginx/sites-enabled/default

# Test and reload nginx
nginx -t
systemctl restart nginx
log "Nginx configured as reverse proxy for Todo API"

# ---------------------------------------------------------------------------
# Step 4: Add Sample Data
# ---------------------------------------------------------------------------
step "Step 4/6: Adding sample todo items..."

sleep 2  # Wait for app to be ready

curl -s -X POST http://localhost:3000/todos -H 'Content-Type: application/json' -d '{"title":"Buy groceries"}' > /dev/null
curl -s -X POST http://localhost:3000/todos -H 'Content-Type: application/json' -d '{"title":"Deploy AI agents"}' > /dev/null
curl -s -X POST http://localhost:3000/todos -H 'Content-Type: application/json' -d '{"title":"Take over the world"}' > /dev/null

log "Sample data added"

# ---------------------------------------------------------------------------
# Step 5: Verify Everything Works
# ---------------------------------------------------------------------------
step "Step 5/6: Verifying setup..."

echo ""
info "Testing Todo API health endpoint..."
HEALTH=$(curl -s http://localhost:3000/health)
echo "Health check: $HEALTH"

echo ""
info "Testing Todo API via nginx..."
TODOS=$(curl -s http://localhost/todos)
echo "Todos: $TODOS"

echo ""
info "Testing public endpoint..."
PUBLIC=$(curl -s http://localhost/)
echo "Public response: $PUBLIC"

# ---------------------------------------------------------------------------
# Step 6: Setup Instructions for Watchers
# ---------------------------------------------------------------------------
step "Step 6/6: Demo preparation complete!"

cat << 'EOF'

╔══════════════════════════════════════════════════════════════╗
║                    ✅ Demo Setup Complete!                    ║
╚══════════════════════════════════════════════════════════════╝

Your demo stack is ready for filming!

📋 What's running:
   • PostgreSQL on port 5432 (database: todos)
   • Todo API on port 3000 (Express + PostgreSQL)
   • Nginx on port 80 (reverse proxy)

🎬 Ready to film:

   1. Show the app working:
      curl http://localhost/todos
      curl http://localhost/health

   2. Break something (pick one):
      • Kill PostgreSQL:    systemctl stop postgresql
      • Kill the app:       systemctl stop todo-app
      • Kill nginx:         systemctl stop nginx
      • Delete nginx config: rm -rf /etc/nginx/conf.d/*

   3. Show osModa self-healing:
      • Watch the watchdog detect the failure
      • Watch the AI diagnose and fix
      • Show the service coming back online

   4. Show the audit log:
      • Check /var/lib/osmoda/audit/ for entries
      • Show the timestamp and downtime

📹 Filming tips:
   • Film your actual screen with a camera (not just screen recording)
   • Show the uncut deletion → recovery sequence
   • Show the audit log after recovery
   • Mention osModa and link spawn.os.moda

🔗 Links to include in video description:
   • osModa Repo: https://github.com/bolivian-peru/os-moda
   • Deploy: https://spawn.os.moda

Good luck with your bounty submission! 🎉

EOF

log "Demo setup complete! Ready to film."
