#!/usr/bin/env bash
set -e

# ==============================================================================
# PCTunnel Relay One-Command VPS Deployment Script
# Target: Ubuntu/Debian VPS (e.g. FindBeatRadar.com subdomain)
# ==============================================================================

DOMAIN="${1:-relay.findbeatradar.com}"

echo "====================================================="
echo "  🚀 Deploying PCTunnel Blind Relay Server           "
echo "  Domain: https://${DOMAIN}                          "
echo "====================================================="

# Check root
if [ "$EUID" -ne 0 ]; then
  echo "[-] Please run as root (sudo ./deploy-relay.sh)"
  exit 1
fi

# 1. Update and install prerequisites
echo "[+] Installing Docker and Compose..."
apt-get update -y
apt-get install -y ca-certificates curl gnupg lsb-release ufw

if ! command -v docker &> /dev/null; then
    mkdir -p /etc/apt/keyrings
    curl -fsSL https://download.docker.com/linux/ubuntu/gpg | gpg --dearmor -o /etc/apt/keyrings/docker.gpg
    echo \
      "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.gpg] https://download.docker.com/linux/ubuntu \
      $(lsb_release -cs) stable" | tee /etc/apt/sources.list.d/docker.list > /dev/null
    apt-get update -y
    apt-get install -y docker-ce docker-ce-cli containerd.io docker-compose-plugin
fi

# 2. Configure Firewall (Only allow 80, 443, 22)
echo "[+] Configuring UFW Firewall..."
ufw allow 22/tcp || true
ufw allow 80/tcp || true
ufw allow 443/tcp || true
ufw --force enable || true

# 3. Create deploy directory
DEPLOY_DIR="/opt/pctunnel"
mkdir -p "${DEPLOY_DIR}"
cd "${DEPLOY_DIR}"

# Export domain env
export RELAY_DOMAIN="${DOMAIN}"
echo "RELAY_DOMAIN=${DOMAIN}" > .env

# Run docker-compose
echo "[+] Building and starting PCTunnel Relay with Caddy SSL..."
docker compose -f deploy/docker-compose.yml up -d --build

echo "====================================================="
echo "  ✅ PCTunnel Relay Successfully Deployed!           "
echo "  WebSocket Endpoint: wss://${DOMAIN}/v1/tunnel      "
echo "  Health Check:       https://${DOMAIN}/health       "
echo "====================================================="
