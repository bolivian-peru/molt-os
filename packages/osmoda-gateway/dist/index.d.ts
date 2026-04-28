#!/usr/bin/env node
/**
 * osModa Gateway — modular runtime, credentials, and agent profiles.
 *
 * On boot:
 *   1. Migration runs if agents.json is missing (absorbs legacy files).
 *   2. agents.json + credentials.json.enc are loaded into in-memory caches.
 *   3. Drivers are registered (claude-code + openclaw).
 *   4. HTTP server exposes /health, /config/*, Telegram webhook.
 *   5. WebSocket server exposes /ws for dashboard chat.
 *   6. SIGHUP reloads agents.json (in-flight sessions keep their snapshot).
 *
 * Endpoints:
 *   GET  /health                — runtime + config health
 *   WS   /ws                    — dashboard chat (Bearer header)
 *   POST /telegram              — Telegram webhook
 *   GET  /config/drivers        — available runtimes
 *   GET  /config/agents, PUT, /:id PATCH/DELETE
 *   GET  /config/credentials, POST, /:id PATCH/DELETE
 *   POST /config/credentials/:id/test, /default
 *   POST /config/reload         — SIGHUP self
 */
export {};
