/**
 * HTTP client for communicating with osmoda-watch over a Unix socket.
 */

import * as http from "node:http";

const WATCH_SOCKET = process.env.OSMODA_WATCH_SOCKET || "/run/osmoda/watch.sock";

export function watchRequest(method: string, reqPath: string, body?: unknown): Promise<string> {
  return new Promise((resolve, reject) => {
    const payload = body ? JSON.stringify(body) : undefined;
    const req = http.request({
      socketPath: WATCH_SOCKET, path: reqPath, method,
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
          reject(new Error(`watch ${method} ${reqPath} returned ${res.statusCode}: ${data}`));
          return;
        }
        resolve(data);
      });
    });
    req.on("error", (e) => reject(new Error(`watch connection failed: ${e.message}`)));
    req.on("timeout", () => { req.destroy(); reject(new Error("watch request timed out")); });
    if (payload) req.write(payload);
    req.end();
  });
}
