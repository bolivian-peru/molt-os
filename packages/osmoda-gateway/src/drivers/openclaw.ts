/**
 * OpenClaw driver — wraps the standalone `openclaw` binary as a child process.
 *
 * OpenClaw is legacy; it's kept as an advanced option for users who rely on
 * its plugin ecosystem. Credential handling is api_key only (Anthropic
 * disabled OAuth for OpenClaw). We write the credential into OpenClaw's
 * auth-profiles.json format before each session, because OpenClaw expects
 * that file at a known path.
 *
 * This driver uses OpenClaw's one-shot run mode (`openclaw run`) and parses
 * its JSON event stream on stdout. If OpenClaw isn't installed on the host,
 * `testCredential` surfaces a clear error.
 */

import { spawn, type ChildProcess } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import * as readline from "node:readline";
import type {
  RuntimeDriver,
  DriverSessionOpts,
  AgentEvent,
  Credential,
  CredentialTestResult,
} from "./types.js";

const OPENCLAW_CANDIDATES = [
  process.env.OPENCLAW_PATH,
  "/opt/openclaw/node_modules/.bin/openclaw",
  "/usr/local/bin/openclaw",
  "/run/current-system/sw/bin/openclaw",
].filter(Boolean) as string[];

function findOpenClawBinary(): string | null {
  for (const p of OPENCLAW_CANDIDATES) {
    try { fs.accessSync(p, fs.constants.X_OK); return p; } catch { /* next */ }
  }
  return null;
}

function writeAuthProfile(agentId: string, cred: Credential): void {
  const dir = path.join("/root/.openclaw/agents", agentId, "agent");
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  const profile = cred.type === "oauth"
    ? { type: "token", provider: cred.provider, token: cred.secret }
    : { type: "api_key", provider: cred.provider, key: cred.secret };
  fs.writeFileSync(path.join(dir, "auth-profiles.json"), JSON.stringify(profile), { mode: 0o600 });
}

export const openClawDriver: RuntimeDriver = {
  name: "openclaw",
  displayName: "OpenClaw (legacy)",
  description:
    "Legacy self-hosted agent engine. API-key only (Anthropic disabled OAuth for OpenClaw). Best when you depend on OpenClaw-specific plugins.",
  supportedProviders: ["anthropic", "openai"],
  supportedAuthTypes: ["api_key"],
  defaultModels: ["claude-opus-4-6", "claude-sonnet-4-6", "gpt-5"],

  async testCredential(cred: Credential): Promise<CredentialTestResult> {
    if (cred.type !== "api_key") {
      return { ok: false, error: `openclaw supports type=api_key only (got ${cred.type})` };
    }
    if (!findOpenClawBinary()) {
      return { ok: false, error: "openclaw binary not installed on this host" };
    }
    if (cred.provider === "anthropic") {
      if (!cred.secret.startsWith("sk-ant-api")) {
        return { ok: false, error: "anthropic api_key should start with sk-ant-api…" };
      }
    } else if (cred.provider === "openai") {
      if (!cred.secret.startsWith("sk-")) {
        return { ok: false, error: "openai api_key should start with sk-…" };
      }
    } else {
      return { ok: false, error: `provider ${cred.provider} not wired for openclaw` };
    }
    // Lightweight validation only — actually calling OpenClaw just to check
    // a credential is expensive. Format + provider match is enough in v1.
    return { ok: true };
  },

  async *startSession(opts: DriverSessionOpts): AsyncGenerator<AgentEvent> {
    const bin = findOpenClawBinary();
    if (!bin) {
      yield { type: "error", code: "no_binary", text: "openclaw binary not found on this host" };
      yield { type: "done" };
      return;
    }

    // OpenClaw reads auth from a file per agent id.
    try { writeAuthProfile(opts.agent.id, opts.credential); }
    catch (e) {
      yield {
        type: "error",
        code: "auth_write_failed",
        text: e instanceof Error ? e.message : String(e),
      };
      yield { type: "done" };
      return;
    }

    const args = [
      "run",
      "--agent", opts.agent.id,
      "--model", opts.model,
      "--mcp-config", opts.mcpConfigPath,
      "--output-format", "json",
      "--message", opts.message,
    ];
    if (opts.sessionId) args.push("--resume", opts.sessionId);

    const proc: ChildProcess = spawn(bin, args, {
      cwd: opts.workingDir || "/root",
      env: { ...process.env, HOME: process.env.HOME || "/root" },
      stdio: ["pipe", "pipe", "pipe"],
    });
    if (opts.abortSignal) {
      opts.abortSignal.addEventListener("abort", () => { proc.kill("SIGTERM"); }, { once: true });
    }

    const rl = readline.createInterface({ input: proc.stdout!, crlfDelay: Infinity });
    let stderrText = "";
    let sessionId: string | undefined;
    let hasOutput = false;
    proc.stderr?.on("data", (d: Buffer) => { stderrText += d.toString(); });

    try {
      for await (const line of rl) {
        if (!line.trim()) continue;
        let ev: any;
        try { ev = JSON.parse(line); } catch { continue; }

        // Normalize OpenClaw's event stream into our AgentEvent shape.
        // OpenClaw emits shapes like:
        //   { type: "event", event: "agent", payload: { stream: "assistant", data: { text, delta } } }
        //   { type: "event", event: "tool_use", payload: { name } }
        //   { type: "event", event: "chat", payload: { state: "final", message: { content: [...] } } }
        if (ev.event === "agent" && ev.payload?.stream === "assistant") {
          const delta = ev.payload.data?.delta;
          if (typeof delta === "string" && delta.length) {
            yield { type: "text", text: delta };
            hasOutput = true;
          }
        } else if (ev.event === "tool_use" && ev.payload?.name) {
          yield { type: "tool_use", name: ev.payload.name };
          hasOutput = true;
        } else if (ev.event === "tool_result") {
          yield { type: "tool_result" };
        } else if (ev.event === "chat" && ev.payload?.state === "final") {
          const content = ev.payload.message?.content;
          if (Array.isArray(content) && !hasOutput) {
            const joined = content
              .filter((c: any) => c.type === "text" && c.text)
              .map((c: any) => c.text)
              .join("\n");
            if (joined) { yield { type: "text", text: joined }; hasOutput = true; }
          }
        } else if (ev.type === "session" && ev.session_id) {
          sessionId = ev.session_id;
          yield { type: "session", sessionId };
        }
      }
    } catch { /* ignore */ }

    const code = await new Promise<number>((resolve) => {
      proc.on("close", (c) => resolve(c ?? 1));
      setTimeout(() => { proc.kill("SIGKILL"); resolve(124); }, 600000);
    });
    if (code !== 0 && !hasOutput && !opts.abortSignal?.aborted) {
      yield { type: "error", text: stderrText.trim().split("\n").pop() || `openclaw exited ${code}` };
    }
    yield { type: "done", sessionId };
  },
};
