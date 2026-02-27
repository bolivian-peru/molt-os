# Authentication

osModa uses the Anthropic API (via OpenClaw) for AI reasoning. You need credentials.

---

## Two types of Anthropic credentials

| Type | Prefix | Source | Billing |
|------|--------|--------|---------|
| **API Key** | `sk-ant-api03-` | [console.anthropic.com](https://console.anthropic.com) | Pay-per-token |
| **OAuth Token** | `sk-ant-oat01-` | `claude setup-token` CLI | Claude Pro/Max subscription |

Both work. The deploy scripts auto-detect which type you have.

---

## API Key (recommended for servers)

Standard pay-per-token key from the Anthropic Console.

1. Go to [console.anthropic.com/settings/keys](https://console.anthropic.com/settings/keys)
2. Create a new key
3. Copy the key (starts with `sk-ant-api03-`)

**Pros:** No expiry, no refresh needed, no subscription restrictions.

**Cons:** Pay-per-token billing. Requires an Anthropic Console account with billing set up.

---

## OAuth Token (subscription-based)

Token from a Claude Pro or Max subscription.

1. Install Claude CLI: `npm install -g @anthropic-ai/claude-code`
2. Run: `claude setup-token`
3. Complete the OAuth flow in your browser
4. Copy the token (starts with `sk-ant-oat01-`)

**Pros:** Uses your existing Claude subscription. No separate billing.

**Cons:** May expire (needs refresh). Some endpoints may reject it with "OAuth authentication is currently not supported" if the service doesn't accept subscription-scoped tokens.

---

## Configuring credentials

### During install

```bash
# install.sh
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh \
  | bash -s -- --api-key sk-ant-api03-YOUR-KEY-HERE
```

### During deploy

```bash
# Pre-stage the key on the server
printf 'sk-ant-api03-...' > /var/lib/osmoda/config/api-key
chmod 600 /var/lib/osmoda/config/api-key

# Then deploy
./scripts/deploy-hetzner.sh 1.2.3.4 ~/.ssh/key
```

### After install

```bash
# Write the key
printf 'sk-ant-api03-...' > /var/lib/osmoda/config/api-key
chmod 600 /var/lib/osmoda/config/api-key

# Also write env file
printf 'ANTHROPIC_API_KEY=sk-ant-api03-...\n' > /var/lib/osmoda/config/env
chmod 600 /var/lib/osmoda/config/env

# Restart gateway
systemctl restart osmoda-gateway  # or: pkill -f "openclaw gateway" && nohup openclaw gateway --port 18789 &
```

---

## How it works internally

The deploy scripts auto-detect your token type and write the correct format to OpenClaw's auth-profiles.json:

**API Key** (`sk-ant-api03-`):
```json
{
  "type": "api_key",
  "provider": "anthropic",
  "key": "sk-ant-api03-..."
}
```

**OAuth Token** (`sk-ant-oat01-`):
```json
{
  "type": "token",
  "provider": "anthropic",
  "token": "sk-ant-oat01-..."
}
```

Each agent has its own auth file at `~/.openclaw/agents/<agentId>/agent/auth-profiles.json` (e.g. `agents/osmoda/agent/auth-profiles.json`).

---

## Troubleshooting

**"No API key found for provider anthropic"**
- Check that `/var/lib/osmoda/config/api-key` exists and is non-empty
- Check that `~/.openclaw/agents/osmoda/agent/auth-profiles.json` exists
- Verify the format matches the examples above

**"OAuth authentication is currently not supported"**
- Anthropic's `sk-ant-oat01-` tokens carry restrictions — they may only work with Claude Code
- Switch to an API key (`sk-ant-api03-`) from [console.anthropic.com](https://console.anthropic.com)

**Token expired**
- OAuth tokens from `claude setup-token` can expire
- Re-run `claude setup-token` and update the key file
- API keys don't expire
