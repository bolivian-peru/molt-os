#!/bin/bash
# Nginx Config Recovery Demonstration Script

set -e  # Exit immediately if a command exits with a non-zero status

# Configuration
NGINX_CONFIG="/etc/nginx/nginx.conf"
BACKUP_CONFIG="/etc/nginx/nginx.conf.bak"

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Intro
echo -e "${GREEN}🎬 Nginx Config Recovery Demo${NC}"
echo "This script demonstrates what happens when an Nginx config is deleted."

# Check if running with sudo
if [[ $EUID -ne 0 ]]; then
   echo -e "${RED}This script must be run as root${NC}" 
   exit 1
fi

# Backup existing config (if not already backed up)
if [ ! -f "$BACKUP_CONFIG" ]; then
    cp "$NGINX_CONFIG" "$BACKUP_CONFIG"
    echo -e "${GREEN}✅ Created initial backup: $BACKUP_CONFIG${NC}"
fi

# Simulate config deletion
echo -e "${RED}🗑️ Deleting Nginx configuration...${NC}"
rm "$NGINX_CONFIG"

# Attempt to restart Nginx (will fail)
echo -e "${RED}❌ Attempting Nginx restart (will fail)...${NC}"
set +e  # Disable exit on error
nginx -t
nginx -s reload
set -e  # Re-enable exit on error

# Recovery Method 1: Restore from backup
echo -e "${GREEN}🔧 Recovering configuration from backup...${NC}"
cp "$BACKUP_CONFIG" "$NGINX_CONFIG"

# Validate and restart
nginx -t
nginx -s reload

echo -e "${GREEN}✅ Configuration recovered successfully!${NC}"
echo "Demo complete. Configuration restored from backup."