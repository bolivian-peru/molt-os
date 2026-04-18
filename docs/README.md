# osModa Documentation

Entry point for the `docs/` tree. Start here, then follow links into the specific doc you need.

*Last curated: 2026-04-18.*

---

## If you're new — start here

1. **[GETTING-STARTED.md](GETTING-STARTED.md)** — install, configure credentials, open the web chat, talk to your server. ~10 minutes.
2. **[ARCHITECTURE.md](ARCHITECTURE.md)** — the mental model: 10 daemons, modular gateway, 91 MCP tools, NixOS atomic rollback, hash-chained audit ledger.
3. **[SECURITY.md](SECURITY.md)** — the honest security brief. The four trust boundaries, what they protect, what they don't.

---

## If you're integrating osModa as an agent service

1. **[SPAWN-API.md](SPAWN-API.md)** — full v1.2.0 public API reference. `POST /api/v1/spawn/:planId` + credentials, idempotency, structured errors, token lifecycle.
2. **[X402.md](X402.md)** — USDC payment protocol (inbound for spawn, outbound for agents paying external APIs).
3. **[AUTH.md](AUTH.md)** — how credentials work in v1.2: encrypted store, OAuth vs API key, multi-provider.
4. **[@osmoda/client (TypeScript SDK)](../packages/osmoda-client/README.md)** — handwritten first-party SDK.

---

## If you want to understand a specific subsystem

- **Agent gateway + runtimes** → [ARCHITECTURE.md § Agent gateway](ARCHITECTURE.md#agent-gateway--modular-runtime-v02)
- **Credentials + OAuth vs API key** → [AUTH.md](AUTH.md)
- **Messaging channels (Telegram / WhatsApp)** → [CHANNELS.md](CHANNELS.md)
- **MCP expansion layer** → [MCP-ECOSYSTEM.md](MCP-ECOSYSTEM.md) (how to add any MCP server as an OS capability)
- **Self-learning system** → [SKILL-LEARNING.md](SKILL-LEARNING.md) (how `osmoda-teachd` generates skills from agent behavior)
- **Multi-perspective risk analysis** → [SWARM-PREDICT.md](SWARM-PREDICT.md) (the `swarm-predict` skill)

---

## If you're contributing or tracking progress

- **[ROADMAP.md](ROADMAP.md)** — what's live, what's next, maturity levels.
- **[STATUS.md](STATUS.md)** — per-component honest assessment. Deeper than the roadmap.
- **Main repo [CLAUDE.md](../CLAUDE.md)** — canonical project overview + conventions (authoritative for coding patterns).

---

## Doc conventions

- **Dates as ISO** (`2026-04-18`) at the top of each doc. Update when you change anything non-trivial.
- **No marketing copy.** If a feature is "Functional" instead of "Solid," the doc should say so. [STATUS.md](STATUS.md) is the reference for where the maturity line falls.
- **No incident reports.** Specific outages go in commit messages and internal memory. This tree is for durable documentation.
- **Internal planning docs** live under `docs/planning/` (gitignored) and don't ship to the public repo.

---

## What's not here anymore (moved / deleted April 2026)

During the April 2026 doc audit we deleted five files that were stale or didn't belong:

| File | Why removed |
|---|---|
| `APP-PERSISTENCE-AUDIT.md` | 2026-03-23 incident report, issues fixed in later commits |
| `FRESH-INSTALL-TEST.md` | 2026-03-02 test run, install flow has changed substantially since |
| `PRODUCTION-ROADMAP.md` | "NOT PRODUCTION READY" emergency plan from March, resolved |
| `HERMES-INTEGRATION-PLAN.md` | Integration plan superseded by the v1.2 modular runtime (which shipped claude-code + openclaw drivers instead of Hermes) |
| `SWARM-PLAN.md` | OpenClaw-specific multi-agent plan; multi-agent is now `agents.json` in [ARCHITECTURE.md](ARCHITECTURE.md) |
| `USE-CASES.md` | Marketing/SEO content; moved to the marketing site, not repo docs |

If you need any of these for historical context, `git log -- docs/<filename>` still shows the history.
