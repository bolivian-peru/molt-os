/**
 * HTTP client for communicating with osmoda-teachd over a Unix socket.
 */

import * as http from "node:http";

const TEACHD_SOCKET = process.env.OSMODA_TEACHD_SOCKET || "/run/osmoda/teachd.sock";

export function teachdRequest(method: string, reqPath: string, body?: unknown): Promise<string> {
  return new Promise((resolve, reject) => {
    const payload = body ? JSON.stringify(body) : undefined;
    const req = http.request({
      socketPath: TEACHD_SOCKET, path: reqPath, method,
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
          reject(new Error(`teachd ${method} ${reqPath} returned ${res.statusCode}: ${data}`));
          return;
        }
        resolve(data);
      });
    });
    req.on("error", (e) => reject(new Error(`teachd connection failed: ${e.message}`)));
    req.on("timeout", () => { req.destroy(); reject(new Error("teachd request timed out")); });
    if (payload) req.write(payload);
    req.end();
  });
}
