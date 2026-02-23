/**
 * HTTP client for communicating with osmoda-mesh over a Unix socket.
 */

import * as http from "node:http";

const MESH_SOCKET = process.env.OSMODA_MESH_SOCKET || "/run/osmoda/mesh.sock";

export function meshRequest(method: string, reqPath: string, body?: unknown): Promise<string> {
  return new Promise((resolve, reject) => {
    const payload = body ? JSON.stringify(body) : undefined;
    const req = http.request({
      socketPath: MESH_SOCKET, path: reqPath, method,
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
          reject(new Error(`mesh ${method} ${reqPath} returned ${res.statusCode}: ${data}`));
          return;
        }
        resolve(data);
      });
    });
    req.on("error", (e) => reject(new Error(`mesh connection failed: ${e.message}`)));
    req.on("timeout", () => { req.destroy(); reject(new Error("mesh request timed out")); });
    if (payload) req.write(payload);
    req.end();
  });
}
