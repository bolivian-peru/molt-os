# Authentication

*Last updated: 2026-04-18. Reflects the v1.2 modular runtime.*

osModa's agent needs a credential to talk to an LLM provider. Starting in v1.2 (April 2026) credentials are a **first-class object** with a lifecycle (create → test → use → rotate → revoke), stored encrypted on disk, and manageable from the dashboard without SSH.

The underlying providers haven't changed — Anthropic's Console API keys and Claude Pro/Max OAuth tokens are the main options, with OpenAI and OpenRouter also supported. What changed is how they're stored and how you switch between them.

---

## The three ways to add a credential

**1. At install time (`--credential` flag, repeatable):**

```bash
# Bring a Claude Pro subscription (flat $20/mo, near-unlimited for heavy use)
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh \
  | sudo bash -s -- \
    --credential "My Claude Pro|anthropic|oauth|$(printf 'sk-ant-oat01-…' | base64)" \
    --default-model claude-opus-4-6

# Pay-per-token API key
curl -fsSL https://raw.githubusercontent.com/bolivian-peru/os-moda/main/scripts/install.sh \
  | sudo bash -s -- \
    --credential "Anthropic Console|anthropic|api_key|$(printf 'sk-ant-api03-…' | base64)"
```

Format: `label|provider|type|base64-secret`. Labels allow spaces and dashes. `provider` ∈ {`anthropic`, `openai`, `openrouter`}. `type` ∈ {`oauth`, `api_key`}.

The legacy `--api-key` flag still works and auto-migrates to a credential on first boot.

**2. From the dashboard (spawn.os.moda or local web UI):**

Open a server → **Engine** tab → Credentials section → **+ Add credential** → fill in label / provider / type / secret → **Add + test**. The probe hits the provider with a 1-token request; status shows `✓ valid` or `✗` with the provider's exact rejection reason.

**3. Via the REST API on the gateway (bearer-authed):**

```bash
TOKEN=$(cat /var/lib/osmoda/config/gateway-token)
curl -X POST http://127.0.0.1:18789/config/credentials \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "label":    "My Claude Pro",
    "provider": "anthropic",
    "type":     "oauth",
    "secret":   "sk-ant-oat01-..."
  }'
# → { "credential": { "id": "cred_...", "secret_preview": "sk-ant-oat01…abcd", … } }
```

---

## Credential types osModa supports today

| Provider | Type | Prefix | Where to get it | Notes |
|---|---|---|---|---|
| `anthropic` | `oauth` | `sk-ant-oat01-` | `npx @anthropic-ai/claude-code setup-token` | Uses your Claude Pro/Max subscription. **Much cheaper for heavy agent use**. Only works through the `claude-code` runtime driver. |
| `anthropic` | `api_key` | `sk-ant-api03-` | [console.anthropic.com/settings/keys](https://console.anthropic.com/settings/keys) | Pay-per-token. Works with both `claude-code` and `openclaw` drivers. |
| `openai` | `api_key` | `sk-` | [platform.openai.com/api-keys](https://platform.openai.com/api-keys) | Works with `openclaw` driver today. |
| `openrouter` | `api_key` | `sk-or-` | [openrouter.ai/keys](https://openrouter.ai/keys) | Routes to many models; works with `openclaw` driver. |

Adding a new provider is a config change — no code required — as long as it's compatible with an existing driver. A new driver (Codex, Bedrock, Vertex) lives as a single file under `packages/osmoda-gateway/src/drivers/`.

---

## OAuth vs API key — which one should you use?

Short version: **OAuth if you have a Claude subscription**, API key otherwise.

Every osModa server that runs a busy agent will comfortably burn $50–200/month in pay-per-token billing. A Claude Pro OAuth token is ~$20/month flat, near-unlimited for sustained agent workloads. That's usually the biggest recurring cost delta in the entire stack.

OAuth caveats:
- Only the `claude-code` runtime driver accepts OAuth. OpenClaw legacy uses API keys only (Anthropic disabled OAuth for OpenClaw).
- Tokens can expire and need refresh via `claude setup-token`.
- Anthropic treats subscription tokens with some API endpoints as restricted. If you see `"OAuth authentication is currently not supported"`, switch the credential type to `api_key` for that agent.

---

## Storage and encryption

Credentials live in two places on disk:

| Path | Contents | Mode | Owner |
|---|---|---|---|
| `/var/lib/osmoda/config/credentials.json.enc` | AES-256-GCM ciphertext of every credential + metadata | `0600` | root |
| `/var/lib/osmoda/config/.credstore-key` | 32-byte master key used to decrypt the above | `0600` | root |

Secrets never leave the gateway. The REST API returns metadata + a 12-char `secret_preview` only. The encryption envelope validates IV + auth tag lengths on read — a corrupted store refuses to load rather than returning garbage.

Legacy files from pre-v1.2 installs are absorbed into the encrypted store on first boot and the old files are deleted:
- `/var/lib/osmoda/config/api-key` → one credential labeled "Migrated Claude Code key"
- `/root/.openclaw/agents/*/agent/auth-profiles.json` → one credential per agent labeled "Migrated OpenClaw (…)"

---

## Assigning a credential to an agent

Each agent has its own credential. The `osmoda` agent (web, full access, Opus) and the `mobile` agent (Telegram/WhatsApp, concise, Sonnet) can use the same credential or different ones.

From the dashboard: **Engine** tab → Agents section → pick the agent → Credential dropdown → select → Save.

From the REST API:
```bash
curl -X PATCH http://127.0.0.1:18789/config/agents/osmoda \
  -H "Authorization: Bearer $(cat /var/lib/osmoda/config/gateway-token)" \
  -H "Content-Type: application/json" \
  -d '{"credential_id":"cred_abc123…", "model":"claude-opus-4-6"}'
```

Saving fires `SIGHUP` on the gateway. In-flight chat sessions keep their original credential snapshot; new sessions pick up the change. Zero WebSocket drops.

---

## Setting a default credential

If you configure multiple credentials, mark one as default so new agents inherit it:

```bash
curl -X POST http://127.0.0.1:18789/config/credentials/cred_abc123…/default \
  -H "Authorization: Bearer $(cat /var/lib/osmoda/config/gateway-token)"
```

Dashboard: Engine tab → Credentials section → **Set as default** button on any credential card.

---

## Testing a credential

Before assigning, verify it actually works:

```bash
curl -X POST http://127.0.0.1:18789/config/credentials/cred_abc123…/test \
  -H "Authorization: Bearer $(cat /var/lib/osmoda/config/gateway-token)"
# → { "ok": true, "model_list": ["claude-opus-4-6", "claude-sonnet-4-6", …] }
# or
# → { "ok": false, "error": "HTTP 401 — invalid credential" }
```

The test hits the provider's `/v1/models` endpoint with a 1-token probe. For Anthropic this returns the list of models your account has access to, which is also how you discover which `model` values are legal for the `agents.json` entry.

---

## Revoking a credential

Removal deletes the encrypted record; agents pointing at the removed credential fail with `"Credential cred_… not found"` until reassigned.

```bash
curl -X DELETE http://127.0.0.1:18789/config/credentials/cred_abc123… \
  -H "Authorization: Bearer $(cat /var/lib/osmoda/config/gateway-token)"
# → 204 No Content
```

Or: dashboard → Engine tab → Credentials → **Remove** button.

The osModa revocation only removes the credential from this server. **You must also revoke the secret at the provider** (Anthropic Console, OpenAI dashboard) if it was exposed.

---

## Troubleshooting

**Agent replies with `"Agent <id> has no credential configured"`**
The agent's `credential_id` is empty or references a credential that was deleted. Open Engine tab → Agents, re-select a credential, save.

**Agent replies with `"Credential cred_… not found"`**
Same root cause — the referenced credential no longer exists. Reassign.

**Test returns `"HTTP 401 — invalid credential"`**
The secret is wrong, revoked at the provider, or was truncated on paste. For Anthropic keys, check that the full `sk-ant-*` string was copied (they're long).

**Test returns `"secret prefix doesn't match type=oauth"`**
Someone configured a Console key (`sk-ant-api03-`) with `type: oauth`, or vice versa. Update the credential and re-test.

**Test returns `"base_url must be https"` or `"resolves to a restricted host"`**
The `base_url` field was set to a private/loopback/metadata address. SSRF defense — set it to a public HTTPS endpoint or leave it blank for the default provider.

**Legacy server hasn't been migrated and still uses `/var/lib/osmoda/config/api-key`**
The migration runs automatically on first boot of the v1.2+ gateway. If you're stuck on v1.1 or earlier, the old file still works. Upgrade via the dashboard → Rebuild (or `nixos-rebuild switch` if self-hosted).

---

## Related docs

- [SECURITY.md](SECURITY.md) — full trust-boundary analysis, including the credential store master key
- [SPAWN-API.md](SPAWN-API.md) — `POST /api/v1/spawn/:planId` `credentials[]` body field for pre-configuring servers at spawn time
- [ARCHITECTURE.md](ARCHITECTURE.md) — driver interface + config layout diagram
