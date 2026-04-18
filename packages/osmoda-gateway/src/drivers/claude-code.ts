/**
 * Claude Code driver — wraps the `claude` CLI in headless streaming mode.
 *
 * Credential handling:
 *  - type=oauth      → CLAUDE_CODE_OAUTH_TOKEN env var (subscription)
 *  - type=api_key    → ANTHROPIC_API_KEY env var + CLI --bare flag
 *
 * The `claude` CLI supports resumable sessions; we pass `--resume <id>` when
 * the caller provides a sessionId.
 */

import { spawn, type ChildProcess } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import * as readline from "node:readline";
import * as https from "node:https";
import type {
  RuntimeDriver,
  DriverSessionOpts,
  AgentEvent,
  Credential,
  CredentialTestResult,
} from "./types.js";

function findClaudeBinary(): string {
  const candidates = [
    process.env.CLAUDE_PATH,
    "/usr/local/bin/claude",
    "/run/current-system/sw/bin/claude",
  ].filter(Boolean) as string[];
  for (const p of candidates) {
    try { fs.accessSync(p, fs.constants.X_OK); return p; } catch { /* next */ }
  }
  return "claude";
}

function buildEnv(cred: Credential): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    HOME: process.env.HOME || "/root",
    NODE_ENV: "production",
  };
  // Scrub stray credentials from inherited env before selectively re-adding.
  delete env.ANTHROPIC_API_KEY;
  delete env.CLAUDE_CODE_OAUTH_TOKEN;
  if (cred.type === "oauth") {
    env.CLAUDE_CODE_OAUTH_TOKEN = cred.secret;
  } else {
    env.ANTHROPIC_API_KEY = cred.secret;
  }
  return env;
}

export const claudeCodeDriver: RuntimeDriver = {
  name: "claude-code",
  displayName: "Claude Code",
  description:
    "Anthropic's official Claude CLI in headless streaming mode. Works with a Claude Pro OAuth subscription or pay-per-token API key.",
  supportedProviders: ["anthropic"],
  supportedAuthTypes: ["oauth", "api_key"],
  defaultModels: ["claude-opus-4-6", "claude-sonnet-4-6", "claude-haiku-4-5"],

  async testCredential(cred: Credential): Promise<CredentialTestResult> {
    if (cred.provider !== "anthropic") {
      return { ok: false, error: `claude-code supports provider=anthropic, got ${cred.provider}` };
    }
    if (!cred.secret || typeof cred.secret !== "string" || cred.secret.length < 20) {
      return { ok: false, error: "secret missing or too short" };
    }
    const prefixOk =
      (cred.type === "oauth" && cred.secret.startsWith("sk-ant-oat")) ||
      (cred.type === "api_key" && cred.secret.startsWith("sk-ant-api"));
    if (!prefixOk) {
      return {
        ok: false,
        error: `secret prefix doesn't match type=${cred.type} (expected sk-ant-${cred.type === "oauth" ? "oat" : "api"}…)`,
      };
    }
    // Real probe: hit /v1/models with the credential.
    try {
      const result = await probeAnthropic(cred);
      return result;
    } catch (e) {
      return { ok: false, error: e instanceof Error ? e.message : String(e) };
    }
  },

  async *startSession(opts: DriverSessionOpts): AsyncGenerator<AgentEvent> {
    const claude = findClaudeBinary();
    const cwd = opts.workingDir || "/root";
    const env = buildEnv(opts.credential);

    const args = [
      "-p",
      "--output-format", "stream-json",
      "--verbose",
      "--model", opts.model,
      "--system-prompt", opts.systemPrompt.slice(0, 10000),
      "--mcp-config", opts.mcpConfigPath,
      "--allowedTools", "Bash,Read,Write,Edit,Glob,Grep,WebFetch,mcp__osmoda__*",
    ];
    if (opts.credential.type === "api_key") args.push("--bare");
    if (opts.sessionId) args.push("--resume", opts.sessionId);
    args.push("--", opts.message);

    let proc: ChildProcess;
    try {
      proc = spawn(claude, args, { cwd, env, stdio: ["pipe", "pipe", "pipe"] });
    } catch (e) {
      yield {
        type: "error",
        code: "spawn_failed",
        text: `Failed to spawn claude: ${e instanceof Error ? e.message : String(e)}`,
      };
      return;
    }

    if (opts.abortSignal) {
      opts.abortSignal.addEventListener("abort", () => { proc.kill("SIGTERM"); }, { once: true });
    }

    const rl = readline.createInterface({ input: proc.stdout!, crlfDelay: Infinity });
    let sessionId: string | undefined;
    let sessionYielded = false;
    let lastTextLen = 0;
    let hasOutput = false;
    let stderrText = "";
    proc.stderr?.on("data", (d: Buffer) => { stderrText += d.toString(); });

    try {
      for await (const line of rl) {
        if (!line.trim()) continue;
        let event: any;
        try { event = JSON.parse(line); } catch { continue; }

        if (event.session_id && !sessionYielded) {
          sessionId = event.session_id;
          sessionYielded = true;
          yield { type: "session", sessionId };
        }

        const t = event.type;
        if (t === "system" && event.subtype === "init") {
          sessionId = event.session_id;
        } else if (t === "assistant") {
          const content = event.message?.content || [];
          if (Array.isArray(content)) {
            for (const block of content) {
              if (block.type === "tool_use" && block.name) {
                yield { type: "tool_use", name: block.name };
                hasOutput = true;
              } else if (block.type === "text" && block.text) {
                const full = block.text;
                if (full.length > lastTextLen) {
                  yield { type: "text", text: full.slice(lastTextLen) };
                  lastTextLen = full.length;
                  hasOutput = true;
                }
              }
            }
          }
        } else if (t === "user") {
          const content = event.message?.content || [];
          if (Array.isArray(content)) {
            for (const block of content) {
              if (block.type === "tool_result") yield { type: "tool_result" };
            }
          }
        } else if (t === "result") {
          if (event.result && typeof event.result === "string" && !hasOutput) {
            yield { type: "text", text: event.result };
          }
          sessionId = event.session_id || sessionId;
        }
      }
    } catch {
      /* readline close or abort — not an error */
    }

    const exitCode = await new Promise<number>((resolve) => {
      proc.on("close", (c) => resolve(c ?? 1));
      setTimeout(() => { proc.kill("SIGKILL"); resolve(124); }, 600000);
    });

    if (exitCode !== 0 && !hasOutput && !opts.abortSignal?.aborted) {
      const errMsg = stderrText.trim().split("\n").pop() || `claude exited with code ${exitCode}`;
      yield { type: "error", text: errMsg };
    }

    yield { type: "done", sessionId };
  },
};

function isSafeBaseUrl(raw: string): { ok: true; url: URL } | { ok: false; reason: string } {
  let u: URL;
  try { u = new URL(raw); } catch { return { ok: false, reason: "invalid_url" }; }
  if (u.protocol !== "https:") return { ok: false, reason: "base_url must be https" };
  const host = u.hostname;
  // Reject literal loopback, link-local, and the AWS/GCP metadata endpoint.
  // Prevents an authed attacker with /config/credentials access from using
  // testCredential as an SSRF primitive against internal services.
  if (/^(localhost|127\.|169\.254\.|0\.0\.0\.0|::1|\[::1\]|\[fc|\[fd|metadata\.google\.internal|169\.254\.169\.254)/i.test(host)) {
    return { ok: false, reason: "base_url resolves to a restricted host" };
  }
  // Block RFC1918 by exact-prefix match on literal IPs. Hostnames that later
  // resolve into RFC1918 would bypass this; for defense in depth we'd also
  // want DNS pinning, but the blast radius is gateway-local only.
  if (/^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/.test(host)) {
    return { ok: false, reason: "base_url resolves to a private network" };
  }
  return { ok: true, url: u };
}

function probeAnthropic(cred: Credential): Promise<CredentialTestResult> {
  return new Promise((resolve) => {
    const headers: Record<string, string> = {
      "anthropic-version": "2023-06-01",
    };
    if (cred.type === "oauth") {
      headers["authorization"] = `Bearer ${cred.secret}`;
    } else {
      headers["x-api-key"] = cred.secret;
    }
    let host = "api.anthropic.com";
    let pathname = "/v1/models";
    if (cred.base_url) {
      const check = isSafeBaseUrl(cred.base_url);
      if (!check.ok) return resolve({ ok: false, error: check.reason });
      host = check.url.host;
      pathname = check.url.pathname.replace(/\/$/, "") + "/v1/models";
    }
    const req = https.request(
      { host, path: pathname, method: "GET", headers, timeout: 10000 },
      (res) => {
        let body = "";
        res.on("data", (c) => (body += c));
        res.on("end", () => {
          if (res.statusCode === 200) {
            try {
              const parsed = JSON.parse(body);
              const models: string[] = Array.isArray(parsed.data)
                ? parsed.data.map((m: any) => m.id).filter(Boolean)
                : [];
              resolve({ ok: true, model_list: models });
            } catch {
              resolve({ ok: true });
            }
          } else if (res.statusCode === 401 || res.statusCode === 403) {
            resolve({ ok: false, error: `HTTP ${res.statusCode} — invalid credential` });
          } else {
            resolve({ ok: false, error: `HTTP ${res.statusCode} ${body.slice(0, 120)}` });
          }
        });
      },
    );
    req.on("error", (e) => resolve({ ok: false, error: e.message }));
    req.on("timeout", () => { req.destroy(); resolve({ ok: false, error: "timeout" }); });
    req.end();
  });
}
