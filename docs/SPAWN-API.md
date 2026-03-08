# Spawn API v1 — x402-Gated Public API

Last updated: 2026-03-06

Programmatic API for spawning osModa servers. Any AI agent pays USDC via x402 and gets a running server with its own AI agent. Agents spawning agents.

---

## Quick start

```bash
# 1. See available plans
curl https://spawn.os.moda/api/v1/plans

# 2. Try to spawn (get 402 with payment details)
curl -X POST https://spawn.os.moda/api/v1/spawn/test

# 3. After x402 payment, you get back:
{
  "order_id": "uuid",
  "api_token": "osk_<64-hex>",
  "server_ip": "1.2.3.4",
  "status": "provisioning",
  "status_url": "https://spawn.os.moda/api/v1/status/<orderId>",
  "chat_url": "wss://spawn.os.moda/api/v1/chat/<orderId>",
  "ssh": "ssh root@1.2.3.4"
}

# 4. Poll status
curl -H "Authorization: Bearer osk_<token>" \
  https://spawn.os.moda/api/v1/status/<orderId>

# 5. Chat with the server's AI agent
wscat -c "wss://spawn.os.moda/api/v1/chat/<orderId>?token=osk_<token>"
```

---

## How x402 works

x402 is an HTTP-native payment protocol (Coinbase standard). No API keys. No signup. Just USDC.

```
Agent → POST /api/v1/spawn/test
Server ← 402 Payment Required
         Headers contain: price ($14.99), network (Base), USDC asset, payTo address

Agent signs USDC transferWithAuthorization (ERC-3009, gasless)
Agent → POST /api/v1/spawn/test
         PAYMENT header with signed authorization

Facilitator (x402.org) verifies + settles on-chain
Server ← 200 OK + server details
```

**Network**: Base Sepolia (testnet) or Base mainnet — depends on server `NETWORK_MODE`.

**Payment packages**: Uses `@x402/express` middleware. Any x402-compatible client (Coinbase SDK `fetch402`, Daydreams agents, custom implementations) works out of the box.

---

## Endpoints

### Free endpoints (no payment)

#### `GET /api/v1/plans`

List available plans with pricing and x402 payment info.

**Response:**
```json
{
  "plans": [
    {
      "id": "test",
      "name": "Solo",
      "description": "1 agent, light tasks",
      "cpu": 2,
      "ram": 4,
      "disk": 40,
      "price_usd": 14.99,
      "tier": "Try it out",
      "endpoint": "https://spawn.os.moda/api/v1/spawn/test",
      "x402": {
        "scheme": "exact",
        "price": "$14.99",
        "network": "eip155:84532",
        "payTo": "0x..."
      }
    }
  ],
  "regions": [
    { "id": "eu-central", "name": "EU Central (Frankfurt)", "flag": "eu" },
    { "id": "eu-north", "name": "EU North (Helsinki)", "flag": "fi" },
    { "id": "us-east", "name": "US East (Virginia)", "flag": "us" },
    { "id": "us-west", "name": "US West (Oregon)", "flag": "us" }
  ],
  "network": "testnet"
}
```

#### `GET /api/v1/status/:orderId`

Check server provisioning status.

**Without auth** — basic info only:
```json
{
  "order_id": "uuid",
  "status": "provisioning",
  "plan": "Solo",
  "created_at": "2026-03-06T12:00:00.000Z"
}
```

**With `Authorization: Bearer osk_<token>`** — full details:
```json
{
  "order_id": "uuid",
  "status": "running",
  "plan": "Solo",
  "created_at": "2026-03-06T12:00:00.000Z",
  "server_ip": "1.2.3.4",
  "server_name": "osmoda-a1b2c3d4",
  "region": "eu-central",
  "ssh": "ssh root@1.2.3.4",
  "chat_url": "wss://spawn.os.moda/api/v1/chat/uuid",
  "price_usd": 14.99
}
```

**Status values**: `pending` → `provisioning` → `running` | `failed`

#### `GET /api/v1/docs`

Full OpenAPI 3.0 specification (machine-readable JSON).

#### `GET /.well-known/agent-card.json`

A2A / ERC-8004 agent discovery card. Returns all plans as skills with x402 pricing, input/output schemas, and endpoint URLs. Used by Daydreams Taskmarket and any A2A-compatible agent for automatic discovery.

---

### x402-gated endpoints (USDC payment required)

All spawn endpoints require x402 USDC payment. Without a valid `PAYMENT` header, the server returns `402 Payment Required` with payment details.

#### `POST /api/v1/spawn/test` — Solo ($14.99/mo)

2 vCPU, 4GB RAM, 40GB SSD. 1 agent, light tasks.

#### `POST /api/v1/spawn/starter` — Pro ($34.99/mo)

4 vCPU, 8GB RAM, 80GB SSD. 2-4 agents, real work.

#### `POST /api/v1/spawn/developer` — Team ($62.99/mo)

8 vCPU, 16GB RAM, 160GB SSD. 5-10 agents, heavy loads.

#### `POST /api/v1/spawn/production` — Scale ($125.99/mo)

16 vCPU, 32GB RAM, 320GB SSD. 10-20+ agents, full fleet.

**Request body** (all fields optional):
```json
{
  "region": "eu-central",
  "ssh_key": "ssh-ed25519 AAAA...",
  "ai_provider": "anthropic",
  "api_key": "sk-ant-..."
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `region` | string | `eu-central` | Server location: `eu-central`, `eu-north`, `us-east`, `us-west` |
| `ssh_key` | string | null | SSH public key (ed25519/RSA/ECDSA). Added to `authorized_keys` |
| `ai_provider` | string | null | `anthropic` or `openai` — AI provider for the server's agent |
| `api_key` | string | null | API key for the provider. Passed via cloud-init, never persisted |

**Response** (after x402 payment verified):
```json
{
  "order_id": "550e8400-e29b-41d4-a716-446655440000",
  "api_token": "osk_a1b2c3d4...",
  "plan": "Pro",
  "price_usd": 34.99,
  "server_ip": "1.2.3.4",
  "status": "provisioning",
  "status_url": "https://spawn.os.moda/api/v1/status/550e8400-...",
  "chat_url": "wss://spawn.os.moda/api/v1/chat/550e8400-...",
  "ssh": "ssh root@1.2.3.4",
  "message": "Server provisioning. osModa installs in 5-10 minutes."
}
```

**`api_token`**: Your secret key for this server. Used for:
- Full status details: `Authorization: Bearer osk_<token>`
- WebSocket chat: `?token=osk_<token>` query param
- Store it securely. It's shown only once.

---

### WebSocket

#### `WS /api/v1/chat/:orderId?token=osk_<token>`

Real-time chat with the spawned server's AI agent.

**Auth**: `token` query parameter with the `osk_` token from the spawn response.

**Messages** (JSON):
```json
// Send to agent
{ "type": "message", "text": "What's the server status?" }

// Receive from agent
{ "type": "message", "text": "All systems operational..." }

// Status updates
{ "type": "status", "agent_connected": true }
```

---

## Authentication

The v1 API uses **two auth mechanisms**:

1. **x402 payment** — for spawn endpoints. The `@x402/express` middleware handles this automatically. You pay once per spawn, and the facilitator settles on-chain.

2. **Bearer token** — for post-spawn operations. The `osk_` token returned in the spawn response authenticates status checks and WebSocket connections.

No API keys. No accounts. No sessions. No cookies. Pay and go.

---

## Rate limits

| Endpoint type | Limit |
|---------------|-------|
| Free endpoints (`/plans`, `/status`, `/docs`, agent card) | 30 req/min per IP |
| Spawn endpoints | 5 req/min per IP |

---

## Error responses

All errors return JSON:

```json
{ "error": "Human-readable error message" }
```

| Status | Meaning |
|--------|---------|
| 400 | Invalid input (bad order ID format, etc.) |
| 402 | Payment required (x402 — includes payment details in headers) |
| 404 | Order or plan not found |
| 429 | Rate limited |
| 500 | Server error (provisioning failed, internal error) |
| 503 | Service unavailable (payment system not active, provisioner offline) |

---

## For agent developers

### Using x402 fetch wrapper

The simplest way to interact with the API from an agent:

```typescript
import { withPayment } from "@x402/fetch";

// Wrap fetch with x402 — handles 402 automatically
const fetch402 = withPayment(fetch, {
  wallet: yourWallet,  // viem wallet client with USDC approval
});

// Spawn a server — payment is automatic
const res = await fetch402("https://spawn.os.moda/api/v1/spawn/test", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({
    region: "eu-central",
    ai_provider: "anthropic",
    api_key: "sk-ant-...",
  }),
});
const data = await res.json();
console.log(data.server_ip, data.api_token);
```

### Using Coinbase CDP SDK

```typescript
import { CdpClient } from "@coinbase/cdp-sdk";

const cdp = new CdpClient();
const account = await cdp.evm.createAccount();

// Fund account with USDC on Base...

const res = await cdp.evm.sendTransaction({
  // x402 handles this automatically through the facilitator
});
```

### Polling for server ready

```typescript
async function waitForServer(orderId, apiToken) {
  while (true) {
    const res = await fetch(`https://spawn.os.moda/api/v1/status/${orderId}`, {
      headers: { Authorization: `Bearer ${apiToken}` },
    });
    const data = await res.json();
    if (data.status === "running") return data;
    if (data.status === "failed") throw new Error("Server failed");
    await new Promise(r => setTimeout(r, 15000)); // poll every 15s
  }
}
```

### WebSocket chat from Node.js

```typescript
import WebSocket from "ws";

const ws = new WebSocket(
  `wss://spawn.os.moda/api/v1/chat/${orderId}?token=${apiToken}`
);

ws.on("open", () => {
  ws.send(JSON.stringify({ type: "message", text: "Deploy a Python API on port 8080" }));
});

ws.on("message", (data) => {
  const msg = JSON.parse(data);
  if (msg.type === "message") console.log("Agent:", msg.text);
});
```

---

## Environment variables

| Variable | Required | Description |
|----------|----------|-------------|
| `ETH_WALLET` | Yes | Receiving wallet address for USDC payments |
| `NETWORK_MODE` | No | `testnet` (default) or `mainnet` |
| `X402_FACILITATOR_URL` | No | Custom facilitator URL (defaults based on NETWORK_MODE) |
| `HETZNER_TOKEN` | Yes | Hetzner Cloud API token for server provisioning |

---

## Existing flows unchanged

The v1 API is a separate Express Router. These are completely untouched:

- `POST /api/spawn` (dashboard crypto flow)
- Stripe checkout + webhooks
- Session cookie auth + magic links
- Heartbeat system
- Dashboard WebSocket (`/api/ws/dash`, `/api/ws/agent`)
- Admin panel

---

## Agent Card schema

The agent card at `/.well-known/agent-card.json` follows the A2A / ERC-8004 pattern:

```json
{
  "name": "osModa Spawn",
  "description": "Spawn dedicated AI-managed NixOS servers...",
  "url": "https://spawn.os.moda",
  "version": "1.0.0",
  "protocol": "A2A",
  "capabilities": { "x402": true, "streaming": true, "websocket": true },
  "skills": [
    {
      "id": "spawn-test",
      "name": "Spawn Solo Server",
      "description": "1 agent, light tasks — 2 vCPU, 4GB RAM, 40GB SSD",
      "endpoint": "https://spawn.os.moda/api/v1/spawn/test",
      "method": "POST",
      "price": {
        "amount": "$14.99",
        "currency": "USDC",
        "protocol": "x402",
        "network": "eip155:84532",
        "asset": "0x036CbD53842c5426634e7929541eC2318f3dCF7e",
        "payTo": "0x..."
      },
      "inputSchema": { "..." },
      "outputSchema": { "..." }
    }
  ],
  "payment": {
    "protocol": "x402",
    "network": "eip155:84532",
    "asset": "0x036...",
    "payTo": "0x..."
  },
  "endpoints": {
    "plans": "https://spawn.os.moda/api/v1/plans",
    "docs": "https://spawn.os.moda/api/v1/docs",
    "status": "https://spawn.os.moda/api/v1/status/{orderId}",
    "chat": "wss://spawn.os.moda/api/v1/chat/{orderId}"
  }
}
```

---

## Daydreams Taskmarket listing

After deploy, register on the Daydreams Taskmarket for agent discovery:

```bash
# Install CLI
npm install -g @lucid-agents/taskmarket

# Register identity (uses agent card)
taskmarket register --url https://spawn.os.moda

# Create service listing
taskmarket list-service \
  --name "osModa Server Spawn" \
  --description "Dedicated AI-managed NixOS servers" \
  --agent-card https://spawn.os.moda/.well-known/agent-card.json

# Verify listing
taskmarket search "server spawn"
```

The agent card at the well-known URL enables automatic discovery by any Daydreams, Lucid, or A2A-compatible agent.

---

## Security notes

- **API tokens**: Generated with `crypto.randomBytes(32)`. Stored as SHA-256 hash (never raw). Shown once at spawn time.
- **Token comparison**: Timing-safe (`crypto.timingSafeEqual` on hashes).
- **Rate limiting**: All endpoints rate-limited per IP.
- **Input validation**: SSH keys regex-checked, AI provider allowlisted, API keys max 256 chars, order IDs UUID-format validated.
- **x402 guard**: If `@x402/express` middleware fails to initialize, spawn endpoints return 503 (no unpaid spawns possible).
- **No email required**: API orders are anonymous. No user account created.
- **API keys**: Passed to server via cloud-init, then deleted from order record.

---

## Network details

| Mode | Chain | Chain ID | CAIP-2 | USDC Contract | Facilitator |
|------|-------|----------|--------|---------------|-------------|
| testnet | Base Sepolia | 84532 | eip155:84532 | `0x036CbD53842c5426634e7929541eC2318f3dCF7e` | x402.org/facilitator |
| mainnet | Base | 8453 | eip155:8453 | `0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913` | x402.org/facilitator |
