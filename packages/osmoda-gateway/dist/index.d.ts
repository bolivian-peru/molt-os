#!/usr/bin/env node
/**
 * osModa Gateway — Claude Code SDK agent runtime.
 *
 * Replaces OpenClaw as the HTTP+WS server for osModa.
 * Connects to the osmoda-mcp-bridge for all 91 system management tools.
 *
 * Endpoints:
 *   GET  /health              — Gateway health check
 *   WS   /ws                  — Dashboard chat (WebSocket)
 *   POST /telegram            — Telegram Bot API webhook
 */
export {};
