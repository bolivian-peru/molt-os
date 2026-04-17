# @osmoda/client

First-party TypeScript client for the [osModa Spawn API](https://spawn.os.moda) (v1.1.0).

## Install

```bash
npm install @osmoda/client
```

Works in Node ≥18 and modern browsers (uses global `fetch` and Web Crypto).

## Quick start

```ts
import { OsmodaClient, OsmodaApiError } from "@osmoda/client";

const client = new OsmodaClient();

// Free: list plans.
const { plans } = await client.listPlans();

// Spawn (x402-gated — wrap fetch yourself, e.g. with @x402/fetch):
const paidClient = new OsmodaClient({
  fetcher: withPayment(fetch, { wallet }),
});

const idempotencyKey = `myapp-${Date.now()}-${crypto.randomUUID()}`;
const spawn = await paidClient.spawn("starter", { region: "eu-central" }, {
  idempotencyKey,           // safe to retry — same key returns same server
});

const bearerClient = new OsmodaClient({ bearer: spawn.api_token });
const server = await bearerClient.waitForRunning(spawn.order_id);
console.log(server.server_ip);
```

## Error handling

Every non-2xx response becomes an `OsmodaApiError` with the structured fields
from the server envelope:

```ts
try {
  await client.spawn("starter", body, { idempotencyKey });
} catch (e) {
  if (e instanceof OsmodaApiError) {
    console.error(e.code, e.message, e.requestId, e.retryAfterSeconds);
  }
}
```

Canonical `code` values (stable across minor versions):
`validation_failed`, `invalid_idempotency_key`, `idempotency_key_reused`,
`plan_not_found`, `order_not_found`, `unauthorized`, `token_expired`,
`token_revoked`, `forbidden`, `rate_limited`, `provisioning_failed`,
`internal_error`, `service_unavailable`.

## Token lifecycle

```ts
import { tokenIdFromToken } from "@osmoda/client";

const client = new OsmodaClient({ bearer: spawn.api_token });
const tokenId = await tokenIdFromToken(spawn.api_token);

const meta = await client.getToken(tokenId);
console.log(meta.expires_at);

await client.revokeToken(tokenId); // 204, token is dead
```

## WebSocket

The chat endpoint is a plain WebSocket at `wss://spawn.os.moda/api/v1/chat/{orderId}?token=osk_...`
— use the runtime's `WebSocket` (browser or `ws` on Node). This SDK
intentionally does not wrap it, to avoid pulling `ws` into your bundle.

See [docs/SPAWN-API.md](../../docs/SPAWN-API.md) for the full protocol
(heartbeat, idle close, backpressure frames).

## Relationship to the OpenAPI spec

This SDK is handwritten to match `GET /api/v1/docs`. It is **not** generated
— by design. It serves as a compile-time regression check: if the runtime
drifts from the types here, the OpenAPI spec is wrong and must be fixed to
match reality.

Version is kept in lockstep with `info.version` in the OpenAPI spec and
`version` in the Agent Card.
