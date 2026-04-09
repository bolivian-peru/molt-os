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

import * as http from "node:http";
import * as fs from "node:fs";
import * as path from "node:path";
import * as https from "node:https";
import { WebSocketServer, WebSocket } from "ws";
import { callAgent } from "./agent.js";
import { SessionStore } from "./sessions.js";

// ── Config ──

interface AgentConfig {
  id: string;
  model: string;
  default?: boolean;
  systemPromptFile?: string;
}

interface GatewayConfig {
  port: number;
  agents: AgentConfig[];
  bindings?: Array<{ agentId: string; channel: string }>;
  telegram?: { botToken: string; allowedUsers?: string[] };
  mcpBridgePath: string;
  authToken?: string;
}

function loadConfig(): GatewayConfig {
  // Try config file
  const configPath = process.env.OSMODA_GATEWAY_CONFIG
    || "/var/lib/osmoda/config/gateway.json";

  let config: Partial<GatewayConfig> = {};
  try {
    config = JSON.parse(fs.readFileSync(configPath, "utf8"));
  } catch {
    console.log(`[gateway] no config at ${configPath}, using defaults`);
  }

  // Try auth token
  let authToken = config.authToken;
  if (!authToken) {
    try {
      authToken = fs.readFileSync("/var/lib/osmoda/config/gateway-token", "utf8").trim();
    } catch { /* no token file */ }
  }

  // Defaults
  return {
    port: config.port || parseInt(process.env.OSMODA_GATEWAY_PORT || "18789", 10),
    agents: config.agents || [
      { id: "osmoda", model: "claude-opus-4-6", default: true },
      { id: "mobile", model: "claude-sonnet-4-6" },
    ],
    bindings: config.bindings || [
      { agentId: "mobile", channel: "telegram" },
    ],
    telegram: config.telegram,
    mcpBridgePath: config.mcpBridgePath
      || process.env.OSMODA_MCP_BRIDGE_PATH
      || "/opt/osmoda/packages/osmoda-mcp-bridge/dist/index.js",
    authToken,
  };
}

function loadSystemPrompt(agent: AgentConfig): string {
  if (agent.systemPromptFile) {
    try {
      return fs.readFileSync(agent.systemPromptFile, "utf8");
    } catch { /* fall through */ }
  }

  // Try default locations
  const candidates = [
    `/root/workspace/SOUL.md`,
    `/var/lib/osmoda/workspace-${agent.id}/SOUL.md`,
    `/opt/osmoda/templates/SOUL.md`,
  ];
  for (const p of candidates) {
    try { return fs.readFileSync(p, "utf8"); } catch { /* next */ }
  }

  return `You are osModa, an AI system administrator with full root access. You manage this NixOS server using 91 tools via MCP.`;
}

function getAgent(config: GatewayConfig, channel?: string): AgentConfig {
  if (channel) {
    const binding = config.bindings?.find(b => b.channel === channel);
    if (binding) {
      const agent = config.agents.find(a => a.id === binding.agentId);
      if (agent) return agent;
    }
  }
  return config.agents.find(a => a.default) || config.agents[0];
}

// ── Main ──

const startTime = Date.now();
const config = loadConfig();
const sessions = new SessionStore();

// Prune expired sessions every 5 minutes
setInterval(() => sessions.prune(), 5 * 60 * 1000);

const server = http.createServer(async (req, res) => {
  const url = new URL(req.url || "/", `http://localhost:${config.port}`);

  // ── Health ──
  if (url.pathname === "/health" || url.pathname === "/api/health") {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({
      status: "ok",
      runtime: "claude-code",
      agents: config.agents.map(a => a.id),
      uptime: Math.floor((Date.now() - startTime) / 1000),
      sessions: sessions.size,
    }));
    return;
  }

  // ── Telegram webhook ──
  if (req.method === "POST" && url.pathname === "/telegram") {
    await handleTelegram(req, res);
    return;
  }

  // ── 404 ──
  res.writeHead(404);
  res.end("Not Found");
});

// ── WebSocket ──

const wss = new WebSocketServer({ server, path: "/ws" });

wss.on("connection", (ws, req) => {
  // Auth check
  if (config.authToken) {
    const auth = req.headers.authorization;
    if (!auth || auth !== `Bearer ${config.authToken}`) {
      ws.close(4001, "Unauthorized");
      return;
    }
  }

  console.log("[gateway] WS client connected");
  let abortController: AbortController | null = null;

  ws.on("message", async (data) => {
    let msg: { type?: string; text?: string; sessionKey?: string };
    try {
      msg = JSON.parse(data.toString());
    } catch {
      ws.send(JSON.stringify({ type: "error", text: "Invalid JSON" }));
      return;
    }

    if (msg.type === "abort" && abortController) {
      abortController.abort();
      abortController = null;
      return;
    }

    if (msg.type !== "chat" || !msg.text) return;

    const agent = getAgent(config, "web");
    const sessionKey = msg.sessionKey || "ws-default";
    const session = sessions.getOrCreate(sessionKey, "web", agent.id);

    abortController = new AbortController();
    const systemPrompt = loadSystemPrompt(agent);

    try {
      for await (const event of callAgent({
        message: msg.text,
        model: agent.model,
        systemPrompt,
        mcpBridgePath: config.mcpBridgePath,
        sessionId: session.claudeSessionId,
        abortSignal: abortController.signal,
      })) {
        if (ws.readyState !== WebSocket.OPEN) break;

        switch (event.type) {
          case "text":
            ws.send(JSON.stringify({ type: "text", text: event.text }));
            break;
          case "tool_use":
            ws.send(JSON.stringify({ type: "tool_use", name: event.name }));
            break;
          case "session":
            if (event.sessionId) {
              sessions.updateClaudeSession(sessionKey, "web", event.sessionId);
            }
            break;
          case "done":
            if (event.sessionId) {
              sessions.updateClaudeSession(sessionKey, "web", event.sessionId);
            }
            ws.send(JSON.stringify({ type: "done" }));
            break;
          case "error":
            ws.send(JSON.stringify({ type: "error", text: event.text }));
            break;
        }
      }
    } catch (e: unknown) {
      if (ws.readyState === WebSocket.OPEN) {
        const message = e instanceof Error ? e.message : String(e);
        ws.send(JSON.stringify({ type: "error", text: message }));
      }
    }

    abortController = null;
  });

  ws.on("close", () => {
    console.log("[gateway] WS client disconnected");
    if (abortController) abortController.abort();
  });
});

// ── Telegram ──

async function handleTelegram(req: http.IncomingMessage, res: http.ServerResponse) {
  if (!config.telegram?.botToken) {
    res.writeHead(503);
    res.end("Telegram not configured");
    return;
  }

  let body = "";
  try {
    for await (const chunk of req) {
      body += chunk;
      if (body.length > 1024 * 1024) { res.writeHead(413); res.end("Too large"); return; }
    }
  } catch {
    res.writeHead(400);
    res.end("Bad request");
    return;
  }

  let update: any;
  try {
    update = JSON.parse(body);
  } catch {
    res.writeHead(400);
    res.end("Invalid JSON");
    return;
  }

  // Respond immediately to Telegram (required within 60s)
  res.writeHead(200);
  res.end("ok");

  const message = update.message;
  if (!message?.text || !message?.chat?.id) return;

  const chatId = String(message.chat.id);
  const username = message.from?.username || "";

  // Check allowed users
  if (config.telegram.allowedUsers?.length) {
    if (!config.telegram.allowedUsers.includes(username) &&
        !config.telegram.allowedUsers.includes(chatId)) {
      return;
    }
  }

  const agent = getAgent(config, "telegram");
  const session = sessions.getOrCreate(chatId, "telegram", agent.id);
  const systemPrompt = loadSystemPrompt(agent);

  let fullText = "";

  try {
    for await (const event of callAgent({
      message: message.text,
      model: agent.model,
      systemPrompt,
      mcpBridgePath: config.mcpBridgePath,
      sessionId: session.claudeSessionId,
    })) {
      if (event.type === "text" && event.text) {
        fullText += event.text;
      }
      if (event.type === "session" && event.sessionId) {
        sessions.updateClaudeSession(chatId, "telegram", event.sessionId);
      }
      if (event.type === "done" && event.sessionId) {
        sessions.updateClaudeSession(chatId, "telegram", event.sessionId);
      }
    }
  } catch (e: unknown) {
    fullText = `Error: ${e instanceof Error ? e.message : String(e)}`;
  }

  if (fullText) {
    await sendTelegram(config.telegram.botToken, chatId, fullText);
  }
}

async function sendTelegram(botToken: string, chatId: string, text: string): Promise<void> {
  const body = JSON.stringify({
    chat_id: chatId,
    text: text.length > 4096 ? text.slice(0, 4093) + "..." : text,
    parse_mode: "Markdown",
  });

  return new Promise((resolve) => {
    const req = https.request({
      hostname: "api.telegram.org",
      path: `/bot${botToken}/sendMessage`,
      method: "POST",
      headers: { "Content-Type": "application/json", "Content-Length": Buffer.byteLength(body) },
    }, (res) => { res.on("data", () => {}); res.on("end", () => resolve()); });
    req.on("error", (e) => {
      console.error("[gateway] Telegram send error:", e.message);
      resolve();
    });
    req.write(body);
    req.end();
  });
}

// ── Start ──

server.listen(config.port, "0.0.0.0", () => {
  console.log(`[gateway] osModa Gateway (Claude Code SDK) listening on port ${config.port}`);
  console.log(`[gateway] Agents: ${config.agents.map(a => `${a.id} (${a.model})`).join(", ")}`);
  console.log(`[gateway] MCP bridge: ${config.mcpBridgePath}`);
});
