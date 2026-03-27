/**
 * HTTP-over-Unix-socket clients for all osModa daemons.
 * Shared between osmoda-bridge (OpenClaw plugin) and osmoda-mcp-bridge (MCP server).
 */

import * as http from "node:http";
import * as child_process from "node:child_process";

// Socket paths (configurable via env)
const SOCKETS = {
  agentd:   process.env.AGENTD_SOCKET   || "/run/osmoda/agentd.sock",
  keyd:     process.env.KEYD_SOCKET     || "/run/osmoda/keyd.sock",
  watch:    process.env.WATCH_SOCKET    || "/run/osmoda/watch.sock",
  routines: process.env.ROUTINES_SOCKET || "/run/osmoda/routines.sock",
  mesh:     process.env.MESH_SOCKET     || "/run/osmoda/mesh.sock",
  mcpd:     process.env.MCPD_SOCKET    || "/run/osmoda/mcpd.sock",
  teachd:   process.env.TEACHD_SOCKET  || "/run/osmoda/teachd.sock",
  voice:    process.env.VOICE_SOCKET   || "/run/osmoda/voice.sock",
};

function socketRequest(
  socketPath: string,
  method: string,
  reqPath: string,
  body?: unknown,
  timeout = 30_000,
): Promise<string> {
  return new Promise((resolve, reject) => {
    const payload = body ? JSON.stringify(body) : undefined;
    const req = http.request({
      socketPath, path: reqPath, method,
      headers: {
        "Content-Type": "application/json",
        ...(payload ? { "Content-Length": String(Buffer.byteLength(payload)) } : {}),
      },
      timeout,
    }, (res) => {
      let data = "";
      res.on("data", (c: Buffer) => { data += c.toString(); });
      res.on("end", () => {
        if (res.statusCode && res.statusCode >= 400) {
          reject(new Error(`${method} ${reqPath} returned ${res.statusCode}: ${data}`));
          return;
        }
        resolve(data);
      });
    });
    req.on("error", (e) => reject(new Error(`Socket ${socketPath} failed: ${e.message}`)));
    req.on("timeout", () => { req.destroy(); reject(new Error(`Socket ${socketPath} timed out`)); });
    if (payload) req.write(payload);
    req.end();
  });
}

// Daemon-specific request functions
export const agentd   = (m: string, p: string, b?: unknown) => socketRequest(SOCKETS.agentd, m, p, b);
export const keyd     = (m: string, p: string, b?: unknown) => socketRequest(SOCKETS.keyd, m, p, b);
export const watch    = (m: string, p: string, b?: unknown) => socketRequest(SOCKETS.watch, m, p, b);
export const routines = (m: string, p: string, b?: unknown) => socketRequest(SOCKETS.routines, m, p, b);
export const mesh     = (m: string, p: string, b?: unknown) => socketRequest(SOCKETS.mesh, m, p, b);
export const mcpd     = (m: string, p: string, b?: unknown) => socketRequest(SOCKETS.mcpd, m, p, b);
export const teachd   = (m: string, p: string, b?: unknown) => socketRequest(SOCKETS.teachd, m, p, b);
export const voice    = (m: string, p: string, b?: unknown) => socketRequest(SOCKETS.voice, m, p, b);

// Shell helpers (same as osmoda-bridge)
export function runShell(cmd: string, timeout = 30000): string {
  try {
    return child_process.execSync(cmd, { timeout, maxBuffer: 1024 * 1024, encoding: "utf-8" });
  } catch (e: any) {
    return `[exit ${e.status || 1}] ${e.stderr || e.message}\n${e.stdout || ""}`;
  }
}

export function runExec(binary: string, args: string[], timeout = 30000): string {
  try {
    return child_process.execFileSync(binary, args, { timeout, maxBuffer: 1024 * 1024, encoding: "utf-8" });
  } catch (e: any) {
    return `[exit ${e.status || 1}] ${e.stderr || e.message}\n${e.stdout || ""}`;
  }
}
