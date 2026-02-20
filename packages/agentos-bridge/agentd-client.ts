/**
 * HTTP client for communicating with agentd over a Unix socket.
 *
 * All agentd communication is HTTP+JSON over a Unix domain socket.
 * This avoids any network exposure — agentd is localhost-only.
 */

import * as http from "node:http";

export class AgentdClient {
  private socketPath: string;

  constructor(socketPath: string) {
    this.socketPath = socketPath;
  }

  async get(path: string): Promise<unknown> {
    return this.request("GET", path);
  }

  async post(path: string, body: unknown): Promise<unknown> {
    return this.request("POST", path, body);
  }

  private request(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<unknown> {
    return new Promise((resolve, reject) => {
      const payload = body ? JSON.stringify(body) : undefined;

      const options: http.RequestOptions = {
        socketPath: this.socketPath,
        path,
        method,
        headers: {
          "Content-Type": "application/json",
          ...(payload ? { "Content-Length": Buffer.byteLength(payload) } : {}),
        },
        timeout: 30_000,
      };

      const req = http.request(options, (res) => {
        const chunks: Buffer[] = [];

        res.on("data", (chunk: Buffer) => {
          chunks.push(chunk);
        });

        res.on("end", () => {
          const raw = Buffer.concat(chunks).toString("utf-8");

          if (res.statusCode && res.statusCode >= 400) {
            reject(
              new Error(
                `agentd ${method} ${path} returned ${res.statusCode}: ${raw}`,
              ),
            );
            return;
          }

          try {
            resolve(JSON.parse(raw));
          } catch {
            // Not JSON — return raw string
            resolve(raw);
          }
        });
      });

      req.on("error", (err: Error) => {
        reject(
          new Error(
            `agentd connection failed (${this.socketPath}): ${err.message}`,
          ),
        );
      });

      req.on("timeout", () => {
        req.destroy();
        reject(new Error(`agentd request timed out: ${method} ${path}`));
      });

      if (payload) {
        req.write(payload);
      }

      req.end();
    });
  }
}
