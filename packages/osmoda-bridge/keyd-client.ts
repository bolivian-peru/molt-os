/**
 * HTTP client for communicating with osmoda-keyd over a Unix socket.
 * Same pattern as agentd-client.ts.
 */

import * as http from "node:http";

const KEYD_SOCKET = process.env.OSMODA_KEYD_SOCKET || "/run/osmoda/keyd.sock";

export function keydRequest(method: string, reqPath: string, body?: unknown): Promise<string> {
  return new Promise((resolve, reject) => {
    const payload = body ? JSON.stringify(body) : undefined;
    const req = http.request({
      socketPath: KEYD_SOCKET, path: reqPath, method,
      headers: {
        "Content-Type": "application/json",
        ...(payload ? { "Content-Length": String(Buffer.byteLength(payload)) } : {}),
      },
      timeout: 30_000,
    }, (res) => {
      let data = "";
      res.on("data", (c: Buffer) => { data += c.toString(); });
      res.on("end", () => {
        if (res.statusCode && res.statusCode >= 400) {
          reject(new Error(`keyd ${method} ${reqPath} returned ${res.statusCode}: ${data}`));
          return;
        }
        resolve(data);
      });
    });
    req.on("error", (e) => reject(new Error(`keyd connection failed: ${e.message}`)));
    req.on("timeout", () => { req.destroy(); reject(new Error("keyd request timed out")); });
    if (payload) req.write(payload);
    req.end();
  });
}
