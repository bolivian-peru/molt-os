# AgentOS First-Boot Setup — Web-based API key wizard
#
# Flow:
#   1. Boot → auto-login → Firefox opens
#   2. If no API key configured: setup wizard serves on :18789
#   3. User connects WiFi, opens console.anthropic.com, gets token
#   4. Pastes token into the setup page
#   5. Setup saves token, starts OpenClaw gateway, redirects to chat
#
# For headless servers: `sudo agentos-setup` CLI or SSH in and write the key file.
{ config, lib, pkgs, ... }:

with lib;

let
  cfg = config.services.agentos;
  apiKeyFile = "${cfg.stateDir}/config/api-key";
  envFile = "${cfg.stateDir}/config/env";
  setupPort = cfg.openclaw.port; # Serve setup on the same port as gateway

  # HTML setup wizard — served when no API key is configured
  setupHtml = pkgs.writeText "agentos-setup.html" ''
    <!DOCTYPE html>
    <html lang="en">
    <head>
      <meta charset="UTF-8">
      <meta name="viewport" content="width=device-width, initial-scale=1.0">
      <title>Thorox Setup</title>
      <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body {
          background: linear-gradient(135deg, #0a0a0f 0%, #111118 50%, #0d0d14 100%);
          color: #e0e0e8;
          font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
          min-height: 100vh;
          display: flex;
          align-items: center;
          justify-content: center;
        }
        .container {
          max-width: 520px;
          width: 100%;
          padding: 48px 40px;
          text-align: center;
        }
        .logo {
          font-size: 32px;
          font-weight: 700;
          letter-spacing: -0.5px;
          margin-bottom: 8px;
          color: #ffffff;
        }
        .subtitle {
          color: #888899;
          font-size: 14px;
          margin-bottom: 48px;
        }
        .step {
          text-align: left;
          margin-bottom: 32px;
        }
        .step-number {
          display: inline-block;
          width: 28px;
          height: 28px;
          border-radius: 50%;
          background: #222233;
          color: #888899;
          text-align: center;
          line-height: 28px;
          font-size: 13px;
          font-weight: 600;
          margin-right: 12px;
        }
        .step-number.active { background: #3b82f6; color: white; }
        .step-text {
          font-size: 15px;
          color: #ccccdd;
        }
        .step-text a {
          color: #60a5fa;
          text-decoration: none;
        }
        .step-text a:hover { text-decoration: underline; }
        .input-group {
          margin-top: 24px;
          text-align: left;
        }
        .input-group label {
          display: block;
          font-size: 13px;
          color: #888899;
          margin-bottom: 8px;
          font-weight: 500;
        }
        input[type="password"] {
          width: 100%;
          padding: 14px 16px;
          background: #1a1a2e;
          border: 1px solid #333344;
          border-radius: 8px;
          color: #e0e0e8;
          font-size: 15px;
          font-family: "JetBrains Mono", monospace;
          outline: none;
          transition: border-color 0.2s;
        }
        input[type="password"]:focus {
          border-color: #3b82f6;
        }
        input[type="password"]::placeholder {
          color: #555566;
        }
        .btn {
          width: 100%;
          padding: 14px;
          margin-top: 24px;
          background: #3b82f6;
          color: white;
          border: none;
          border-radius: 8px;
          font-size: 15px;
          font-weight: 600;
          cursor: pointer;
          transition: background 0.2s;
        }
        .btn:hover { background: #2563eb; }
        .btn:disabled { background: #333344; cursor: not-allowed; }
        .error {
          color: #f87171;
          font-size: 13px;
          margin-top: 12px;
          display: none;
        }
        .success {
          color: #4ade80;
          font-size: 15px;
          margin-top: 24px;
          display: none;
        }
        .spinner {
          display: none;
          margin: 24px auto 0;
          width: 32px;
          height: 32px;
          border: 3px solid #333344;
          border-top: 3px solid #3b82f6;
          border-radius: 50%;
          animation: spin 0.8s linear infinite;
        }
        @keyframes spin { to { transform: rotate(360deg); } }
      </style>
    </head>
    <body>
      <div class="container">
        <div class="logo">Thorox</div>
        <div class="subtitle">Your server has a brain now</div>

        <div class="step" id="wifi-hint" style="display:none; margin-bottom: 24px; padding: 12px 16px; background: #1a1a2e; border-radius: 8px; border: 1px solid #333344;">
          <span class="step-text" style="font-size: 13px; color: #888899;">
            Need WiFi? Press <strong style="color:#60a5fa">Super+T</strong> to open a terminal, then run <code style="background:#222233; padding:2px 6px; border-radius:4px;">nmtui</code>
          </span>
        </div>

        <div class="step">
          <span class="step-number active">1</span>
          <span class="step-text">
            Open <a href="https://console.anthropic.com/settings/keys" target="_blank">console.anthropic.com</a> and create an API key
          </span>
        </div>

        <div class="step">
          <span class="step-number">2</span>
          <span class="step-text">Paste your API key below</span>
        </div>

        <div class="input-group">
          <label>Anthropic API Key</label>
          <input type="password" id="apiKey" placeholder="sk-ant-..." autocomplete="off" autofocus>
        </div>

        <div class="error" id="error"></div>

        <button class="btn" id="submit" onclick="submitKey()">
          Activate Thorox
        </button>

        <div class="spinner" id="spinner"></div>
        <div class="success" id="success">
          Connected. Starting Thorox...
        </div>
      </div>

      <script>
        const input = document.getElementById('apiKey');
        const btn = document.getElementById('submit');
        const error = document.getElementById('error');
        const success = document.getElementById('success');
        const spinner = document.getElementById('spinner');
        const wifiHint = document.getElementById('wifi-hint');

        // Show WiFi hint if offline
        if (!navigator.onLine) wifiHint.style.display = 'block';
        window.addEventListener('online', () => wifiHint.style.display = 'none');
        window.addEventListener('offline', () => wifiHint.style.display = 'block');

        input.addEventListener('keydown', (e) => {
          if (e.key === 'Enter') submitKey();
        });

        async function submitKey() {
          const key = input.value.trim();
          error.style.display = 'none';

          if (!key) {
            error.textContent = 'Please enter your API key.';
            error.style.display = 'block';
            return;
          }

          btn.disabled = true;
          btn.textContent = 'Verifying...';
          spinner.style.display = 'block';

          try {
            const res = await fetch('/setup/activate', {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ key: key })
            });
            const data = await res.json();

            if (data.ok) {
              spinner.style.display = 'none';
              success.style.display = 'block';
              btn.style.display = 'none';
              input.style.display = 'none';

              // Poll until gateway is ready, then redirect
              const poll = setInterval(async () => {
                try {
                  const r = await fetch('/', { method: 'HEAD' });
                  if (r.ok) { clearInterval(poll); window.location.href = '/'; }
                } catch (_) {}
              }, 1000);
            } else {
              throw new Error(data.error || 'Setup failed');
            }
          } catch (err) {
            spinner.style.display = 'none';
            error.textContent = err.message;
            error.style.display = 'block';
            btn.disabled = false;
            btn.textContent = 'Activate Thorox';
          }
        }
      </script>
    </body>
    </html>
  '';

  # Setup web server — tiny Node.js server that serves the wizard and handles activation
  setupServer = pkgs.writeShellScript "agentos-setup-server" ''
    set -euo pipefail

    KEY_FILE="${apiKeyFile}"
    ENV_FILE="${envFile}"
    PORT="${toString setupPort}"
    HTML_FILE="${setupHtml}"

    mkdir -p "$(dirname "$KEY_FILE")"
    mkdir -p "$(dirname "$ENV_FILE")"

    # If key exists, exit immediately (let gateway start instead)
    if [ -f "$KEY_FILE" ] && [ -s "$KEY_FILE" ]; then
      echo "API key found, setup not needed."
      exit 0
    fi

    echo "Starting Thorox setup wizard on port $PORT..."

    ${pkgs.nodejs_22}/bin/node -e "
    const http = require('http');
    const fs = require('fs');
    const path = require('path');

    const html = fs.readFileSync('$HTML_FILE', 'utf-8');
    const keyFile = '$KEY_FILE';
    const envFile = '$ENV_FILE';

    const server = http.createServer((req, res) => {
      if (req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'text/html' });
        res.end(html);
        return;
      }

      if (req.method === 'POST' && req.url === '/setup/activate') {
        let body = '';
        req.on('data', c => body += c);
        req.on('end', () => {
          try {
            const { key } = JSON.parse(body);
            if (!key || key.length < 10) {
              res.writeHead(400, { 'Content-Type': 'application/json' });
              res.end(JSON.stringify({ ok: false, error: 'Invalid key' }));
              return;
            }

            // Save the key
            fs.mkdirSync(path.dirname(keyFile), { recursive: true });
            fs.writeFileSync(keyFile, key, { mode: 0o600 });

            // Generate env file for the gateway
            fs.writeFileSync(envFile, 'ANTHROPIC_API_KEY=' + key + '\n', { mode: 0o600 });

            res.writeHead(200, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ ok: true }));

            // Shutdown setup server and start gateway
            console.log('API key saved. Starting gateway...');
            setTimeout(() => {
              const { execSync } = require('child_process');
              try {
                execSync('systemctl start agentos-gateway.service', { stdio: 'inherit' });
              } catch (e) {
                console.error('Failed to start gateway:', e.message);
              }
              process.exit(0);
            }, 1000);
          } catch (e) {
            res.writeHead(500, { 'Content-Type': 'application/json' });
            res.end(JSON.stringify({ ok: false, error: e.message }));
          }
        });
        return;
      }

      res.writeHead(404);
      res.end('Not found');
    });

    server.listen(parseInt('$PORT'), '127.0.0.1', () => {
      console.log('Setup wizard running at http://localhost:' + '$PORT');
    });
    "
  '';

  # CLI setup command for headless/SSH usage
  setupCommand = pkgs.writeShellScriptBin "agentos-setup" ''
    set -euo pipefail

    KEY_FILE="${apiKeyFile}"
    ENV_FILE="${envFile}"

    mkdir -p "$(dirname "$KEY_FILE")"
    mkdir -p "$(dirname "$ENV_FILE")"

    echo ""
    echo "Thorox Setup"
    echo "============"

    if [ -f "$KEY_FILE" ] && [ -s "$KEY_FILE" ]; then
      EXISTING=$(head -c 15 "$KEY_FILE")
      echo "Current key: ''${EXISTING}..."
      echo -n "Replace? [y/N]: "
      read -r REPLACE
      if [ "$REPLACE" != "y" ] && [ "$REPLACE" != "Y" ]; then
        echo "Keeping existing key."
        exit 0
      fi
    fi

    echo -n "Anthropic API Key: "
    read -r API_KEY

    if [ -z "$API_KEY" ]; then
      echo "No key provided. Aborted."
      exit 1
    fi

    echo "$API_KEY" > "$KEY_FILE"
    chmod 600 "$KEY_FILE"

    echo "ANTHROPIC_API_KEY=$API_KEY" > "$ENV_FILE"
    chmod 600 "$ENV_FILE"

    echo "API key saved."

    if systemctl is-active --quiet agentos-gateway; then
      echo "Restarting gateway..."
      systemctl restart agentos-gateway
    elif systemctl is-active --quiet agentos-setup-wizard; then
      echo "Stopping setup wizard, starting gateway..."
      systemctl stop agentos-setup-wizard
      systemctl start agentos-gateway
    else
      echo "Starting gateway..."
      systemctl start agentos-gateway
    fi
    echo "Done."
  '';
in {
  config = mkIf cfg.enable {
    environment.systemPackages = [ setupCommand ];

    # Setup wizard web server — serves on gateway port when no API key exists
    systemd.services.agentos-setup-wizard = {
      description = "AgentOS Setup Wizard (first-boot)";
      wantedBy = [ "multi-user.target" ];

      # Only start if no API key configured
      unitConfig = {
        ConditionPathExists = "!${apiKeyFile}";
      };

      # Conflict with gateway — only one can bind the port
      conflicts = [ "agentos-gateway.service" ];
      before = [ "agentos-gateway.service" ];

      serviceConfig = {
        Type = "simple";
        ExecStart = setupServer;
        Restart = "on-failure";
        RestartSec = 3;
        RuntimeDirectory = "agentos";
        StateDirectory = "agentos";
      };
    };

    # Gateway should only start if API key exists
    systemd.services.agentos-gateway = mkIf cfg.openclaw.enable {
      unitConfig = {
        ConditionPathExists = [ apiKeyFile ];
      };
      serviceConfig = {
        EnvironmentFile = [ "-${envFile}" ];
      };
    };

    # Activation: generate env file from existing key (for rebuilds)
    system.activationScripts.agentos-api-env = ''
      KEY_FILE="${apiKeyFile}"
      ENV_FILE="${envFile}"
      mkdir -p "$(dirname "$ENV_FILE")"
      if [ -f "$KEY_FILE" ] && [ -s "$KEY_FILE" ]; then
        echo "ANTHROPIC_API_KEY=$(cat "$KEY_FILE")" > "$ENV_FILE"
        chmod 600 "$ENV_FILE"
      fi
    '';
  };
}
