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

import * as http from "node:http";
import * as fs from "node:fs";
import * as path from "node:path";
import * as https from "node:https";
import { WebSocketServer, WebSocket } from "ws";
import { SessionStore } from "./sessions.js";
import { ConfigCache } from "./config.js";
import { runMigrationIfNeeded } from "./migrate.js";
import { getCredential, loadCredentials } from "./credentials.js";
import { getDriver, listDrivers } from "./drivers/index.js";
import type { AgentProfile, AgentEvent } from "./drivers/types.js";
import { handleConfigRequest } from "./config-api.js";

// ── Gateway config (NOT to be confused with agent config) ───────────────

interface GatewayEnv {
  port: number;
  authToken: string | null;
  mcpBridgePath: string;
  telegramBotToken: string;
  telegramAllowedUsers: string[];
}

function loadGatewayEnv(): GatewayEnv {
  let authToken: string | null = null;
  try { authToken = fs.readFileSync("/var/lib/osmoda/config/gateway-token", "utf8").trim(); }
  catch { /* no token file */ }
  return {
    port: parseInt(process.env.OSMODA_GATEWAY_PORT || "18789", 10),
    authToken,
    mcpBridgePath: process.env.OSMODA_MCP_BRIDGE_PATH
      || "/opt/osmoda/packages/osmoda-mcp-bridge/dist/index.js",
    telegramBotToken: process.env.TELEGRAM_BOT_TOKEN || "",
    telegramAllowedUsers: (process.env.TELEGRAM_ALLOWED_USERS || "").split(",").map(s => s.trim()).filter(Boolean),
  };
}

// ── MCP config (per-agent, because paths may differ per profile) ────────

function buildMcpConfig(mcpBridgePath: string): object {
  return {
    mcpServers: {
      osmoda: {
        command: "node",
        args: [mcpBridgePath],
        env: {
          AGENTD_SOCKET: process.env.OSMODA_SOCKET || "/run/osmoda/agentd.sock",
          KEYD_SOCKET: process.env.OSMODA_KEYD_SOCKET || "/run/osmoda/keyd.sock",
          WATCH_SOCKET: process.env.OSMODA_WATCH_SOCKET || "/run/osmoda/watch.sock",
          ROUTINES_SOCKET: process.env.OSMODA_ROUTINES_SOCKET || "/run/osmoda/routines.sock",
          MESH_SOCKET: process.env.OSMODA_MESH_SOCKET || "/run/osmoda/mesh.sock",
          MCPD_SOCKET: process.env.OSMODA_MCPD_SOCKET || "/run/osmoda/mcpd.sock",
          TEACHD_SOCKET: process.env.OSMODA_TEACHD_SOCKET || "/run/osmoda/teachd.sock",
          VOICE_SOCKET: process.env.OSMODA_VOICE_SOCKET || "/run/osmoda/voice.sock",
        },
      },
    },
  };
}

let _mcpConfigPath: string | null = null;
function getMcpConfigPath(mcpBridgePath: string): string {
  if (_mcpConfigPath) return _mcpConfigPath;
  const stable = "/var/lib/osmoda/config/mcp-bridge.json";
  const fallback = `/tmp/osmoda-mcp-${process.pid}.json`;
  const body = JSON.stringify(buildMcpConfig(mcpBridgePath), null, 2);
  try {
    fs.mkdirSync(path.dirname(stable), { recursive: true });
    fs.writeFileSync(stable, body);
    fs.chmodSync(stable, 0o644);
    _mcpConfigPath = stable;
  } catch {
    fs.writeFileSync(fallback, body);
    _mcpConfigPath = fallback;
  }
  return _mcpConfigPath;
}

// ── System prompt (SOUL.md etc) ─────────────────────────────────────────

function loadSystemPrompt(agent: AgentProfile): string {
  if (agent.system_prompt_file) {
    try { return fs.readFileSync(agent.system_prompt_file, "utf8"); } catch { /* fall through */ }
  }
  const candidates = [
    agent.profile_dir ? path.join(agent.profile_dir, "SOUL.md") : null,
    `/root/workspace/SOUL.md`,
    `/var/lib/osmoda/workspace-${agent.id}/SOUL.md`,
    `/opt/osmoda/templates/agents/${agent.id}/SOUL.md`,
    `/opt/osmoda/templates/SOUL.md`,
  ].filter(Boolean) as string[];
  for (const p of candidates) {
    try { return fs.readFileSync(p, "utf8"); } catch { /* next */ }
  }
  return `You are osModa, an AI system administrator with full root access. You manage this NixOS server using 92 tools via MCP.`;
}

// ── Boot ────────────────────────────────────────────────────────────────

const startTime = Date.now();
const migrationReport = runMigrationIfNeeded();
if (migrationReport.ran) {
  console.log(`[gateway] migration: ran=${migrationReport.ran} creds=${migrationReport.imported_credentials} agents=${migrationReport.created_agents} runtime=${migrationReport.detected_runtime}`);
  for (const note of migrationReport.notes) console.log(`[gateway]   ${note}`);
}

const env = loadGatewayEnv();
const cache = new ConfigCache();
const sessions = new SessionStore();

// Prune expired sessions every 5 minutes.
setInterval(() => sessions.prune(), 5 * 60 * 1000);

// SIGHUP → in-memory config reload. Does not interrupt active sessions.
process.on("SIGHUP", () => {
  try {
    cache.reload();
    console.log(`[gateway] SIGHUP: config reloaded (${cache.current().agents.length} agents)`);
  } catch (e) {
    console.error("[gateway] SIGHUP reload failed:", e instanceof Error ? e.message : String(e));
  }
});

function reloadSelf(): void {
  cache.reload();
  // Also kick the actual OS signal in case upstream expects it.
  try { process.kill(process.pid, "SIGHUP"); } catch { /* ignore */ }
}

// ── HTTP server ─────────────────────────────────────────────────────────

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url || "/", `http://localhost:${env.port}`);

  // Config API — may handle + respond.
  if (await handleConfigRequest(req, res, url, { cache, authToken: env.authToken, reloadSelf })) {
    return;
  }

  // Health
  if (url.pathname === "/health" || url.pathname === "/api/health") {
    const creds = loadCredentials();
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({
      status: "ok",
      runtime: "modular",
      drivers: listDrivers().map(d => d.name),
      agents: cache.current().agents.map(a => ({
        id: a.id, runtime: a.runtime, model: a.model, enabled: a.enabled,
      })),
      credentials_count: creds.credentials.length,
      uptime: Math.floor((Date.now() - startTime) / 1000),
      sessions: sessions.size,
    }));
    return;
  }

  // Telegram webhook
  if (req.method === "POST" && url.pathname === "/telegram") {
    await handleTelegram(req, res);
    return;
  }

  res.writeHead(404);
  res.end("Not Found");
});

// ── WebSocket ──────────────────────────────────────────────────────────

const wss = new WebSocketServer({ server, path: "/ws" });

wss.on("connection", (ws, req) => {
  if (env.authToken) {
    const auth = req.headers.authorization;
    if (!auth || auth !== `Bearer ${env.authToken}`) {
      ws.close(4001, "Unauthorized");
      return;
    }
  }
  let abortController: AbortController | null = null;

  ws.on("message", async (data) => {
    let msg: { type?: string; text?: string; sessionKey?: string; agentId?: string };
    try { msg = JSON.parse(data.toString()); } catch {
      ws.send(JSON.stringify({ type: "error", text: "Invalid JSON" }));
      return;
    }
    if (msg.type === "abort" && abortController) { abortController.abort(); abortController = null; return; }
    if (msg.type !== "chat" || !msg.text) return;

    const agent = msg.agentId ? cache.findAgent(msg.agentId) : cache.agentForChannel("web");
    if (!agent) { ws.send(JSON.stringify({ type: "error", text: "No agent configured" })); return; }
    if (!agent.credential_id) {
      ws.send(JSON.stringify({ type: "error", text: `Agent ${agent.id} has no credential configured` }));
      return;
    }
    const credential = getCredential(agent.credential_id);
    if (!credential) {
      ws.send(JSON.stringify({ type: "error", text: `Credential ${agent.credential_id} not found` }));
      return;
    }
    const driver = getDriver(agent.runtime);
    if (!driver) {
      ws.send(JSON.stringify({ type: "error", text: `Unknown runtime ${agent.runtime}` }));
      return;
    }

    const sessionKey = msg.sessionKey || "ws-default";
    const session = sessions.getOrCreate(sessionKey, "web", agent.id);
    abortController = new AbortController();
    const systemPrompt = loadSystemPrompt(agent);

    try {
      for await (const event of driver.startSession({
        agent, credential, model: agent.model, systemPrompt,
        mcpConfigPath: getMcpConfigPath(env.mcpBridgePath),
        message: msg.text,
        sessionId: session.claudeSessionId,
        abortSignal: abortController.signal,
        workingDir: agent.profile_dir,
      })) {
        if (ws.readyState !== WebSocket.OPEN) break;
        pipeEvent(ws, event, sessionKey);
      }
    } catch (e: any) {
      if (ws.readyState === WebSocket.OPEN) {
        ws.send(JSON.stringify({ type: "error", text: e?.message || String(e) }));
      }
    }
    abortController = null;
  });

  ws.on("close", () => { if (abortController) abortController.abort(); });
});

function pipeEvent(ws: WebSocket, event: AgentEvent, sessionKey: string): void {
  switch (event.type) {
    case "text":
      ws.send(JSON.stringify({ type: "text", text: event.text })); break;
    case "tool_use":
      ws.send(JSON.stringify({ type: "tool_use", name: event.name })); break;
    case "tool_result":
      ws.send(JSON.stringify({ type: "tool_result" })); break;
    case "thinking":
      ws.send(JSON.stringify({ type: "thinking", text: event.text })); break;
    case "session":
      if (event.sessionId) sessions.updateClaudeSession(sessionKey, "web", event.sessionId);
      break;
    case "done":
      if (event.sessionId) sessions.updateClaudeSession(sessionKey, "web", event.sessionId);
      ws.send(JSON.stringify({ type: "done" }));
      break;
    case "error":
      ws.send(JSON.stringify({ type: "error", text: event.text, code: event.code }));
      break;
  }
}

// ── Telegram ───────────────────────────────────────────────────────────

async function handleTelegram(req: http.IncomingMessage, res: http.ServerResponse): Promise<void> {
  if (!env.telegramBotToken) {
    res.writeHead(503); res.end("Telegram not configured"); return;
  }
  let body = "";
  try {
    for await (const chunk of req) {
      body += chunk;
      if (body.length > 1024 * 1024) { res.writeHead(413); res.end("Too large"); return; }
    }
  } catch { res.writeHead(400); res.end("Bad request"); return; }

  let update: any;
  try { update = JSON.parse(body); } catch { res.writeHead(400); res.end("Invalid JSON"); return; }
  res.writeHead(200); res.end("ok");  // ack immediately

  const message = update.message;
  if (!message?.text || !message?.chat?.id) return;
  const chatId = String(message.chat.id);
  const username = message.from?.username || "";
  if (env.telegramAllowedUsers.length &&
      !env.telegramAllowedUsers.includes(username) &&
      !env.telegramAllowedUsers.includes(chatId)) return;

  const agent = cache.agentForChannel("telegram");
  if (!agent || !agent.credential_id) {
    await sendTelegram(env.telegramBotToken, chatId, "Telegram agent not configured. Visit the dashboard → Settings → Agents."); return;
  }
  const credential = getCredential(agent.credential_id);
  if (!credential) {
    await sendTelegram(env.telegramBotToken, chatId, "Telegram agent's credential is missing. Visit Settings → Credentials."); return;
  }
  const driver = getDriver(agent.runtime);
  if (!driver) {
    await sendTelegram(env.telegramBotToken, chatId, `Agent runtime ${agent.runtime} not available.`); return;
  }

  const session = sessions.getOrCreate(chatId, "telegram", agent.id);
  const systemPrompt = loadSystemPrompt(agent);
  let fullText = "";

  try {
    for await (const event of driver.startSession({
      agent, credential, model: agent.model, systemPrompt,
      mcpConfigPath: getMcpConfigPath(env.mcpBridgePath),
      message: message.text,
      sessionId: session.claudeSessionId,
      workingDir: agent.profile_dir,
    })) {
      if (event.type === "text" && event.text) fullText += event.text;
      if (event.type === "session" && event.sessionId) {
        sessions.updateClaudeSession(chatId, "telegram", event.sessionId);
      }
      if (event.type === "done" && event.sessionId) {
        sessions.updateClaudeSession(chatId, "telegram", event.sessionId);
      }
    }
  } catch (e: any) { fullText = `Error: ${e?.message || String(e)}`; }

  if (fullText) await sendTelegram(env.telegramBotToken, chatId, fullText);
}

async function sendTelegram(botToken: string, chatId: string, text: string): Promise<void> {
  const body = JSON.stringify({
    chat_id: chatId,
    text: text.length > 4096 ? text.slice(0, 4093) + "..." : text,
    parse_mode: "Markdown",
  });
  return new Promise((resolve) => {
    const req = https.request(
      {
        hostname: "api.telegram.org",
        path: `/bot${botToken}/sendMessage`,
        method: "POST",
        headers: { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(body) },
      },
      (r) => { r.on("data", () => {}); r.on("end", () => resolve()); },
    );
    req.on("error", (e) => { console.error("[gateway] Telegram send error:", e.message); resolve(); });
    req.write(body); req.end();
  });
}

// ── Start ───────────────────────────────────────────────────────────────

server.listen(env.port, "0.0.0.0", () => {
  console.log(`[gateway] osModa Gateway (modular) listening on port ${env.port}`);
  console.log(`[gateway] drivers: ${listDrivers().map(d => d.name).join(", ")}`);
  const current = cache.current();
  console.log(`[gateway] agents: ${current.agents.map(a => `${a.id}(${a.runtime}/${a.model})`).join(", ") || "<none>"}`);
  console.log(`[gateway] credentials: ${loadCredentials().credentials.length}`);
});
