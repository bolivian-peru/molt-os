/**
 * Claude Code agent wrapper — spawns claude CLI in headless mode with MCP tools.
 *
 * Uses `claude --print --output-format stream-json` for programmatic access.
 * The CLI connects to the osmoda-mcp-bridge MCP server for all 91 system tools.
 */

import { spawn, type ChildProcess } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import * as readline from "node:readline";

export interface AgentCallOptions {
  message: string;
  model: string;
  systemPrompt: string;
  mcpBridgePath: string;
  sessionId?: string;
  cwd?: string;
  abortSignal?: AbortSignal;
}

export interface AgentEvent {
  type: "text" | "tool_use" | "tool_result" | "done" | "error" | "session";
  text?: string;
  name?: string;
  sessionId?: string;
}

/** Resolve claude binary path */
function findClaude(): string {
  const candidates = [
    "/usr/local/bin/claude",
    "/root/.nix-profile/bin/claude",
    // npm global
    process.env.CLAUDE_PATH,
    // Local install in gateway
    path.resolve(import.meta.dirname || __dirname, "../node_modules/.bin/claude"),
  ].filter(Boolean) as string[];

  for (const p of candidates) {
    try {
      fs.accessSync(p, fs.constants.X_OK);
      return p;
    } catch { /* next */ }
  }
  return "claude"; // hope it's on PATH
}

/** Build MCP config JSON for Claude Code */
function buildMcpConfig(mcpBridgePath: string): string {
  const config = {
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
  return JSON.stringify(config);
}

/**
 * Call the Claude Code agent with a user message.
 * Spawns `claude --print --output-format stream-json` and yields streaming events.
 */
export async function* callAgent(opts: AgentCallOptions): AsyncGenerator<AgentEvent> {
  const claude = findClaude();
  const cwd = opts.cwd || "/root";

  // Write temporary MCP config
  const mcpConfigPath = `/tmp/osmoda-mcp-${Date.now()}.json`;
  fs.writeFileSync(mcpConfigPath, buildMcpConfig(opts.mcpBridgePath));

  const args = [
    "--print",
    "--output-format", "stream-json",
    "--model", opts.model,
    "--system-prompt", opts.systemPrompt,
    "--dangerously-skip-permissions",
    "--mcp-config", mcpConfigPath,
    "--no-session-persistence",
    "--max-turns", "30",
  ];

  // Resume session if we have one
  if (opts.sessionId) {
    args.push("--resume", opts.sessionId);
  }

  // The prompt goes at the end
  args.push("--", opts.message);

  let proc: ChildProcess;
  try {
    proc = spawn(claude, args, {
      cwd,
      env: {
        ...process.env,
        HOME: "/root",
        NODE_ENV: "production",
      },
      stdio: ["pipe", "pipe", "pipe"],
    });
  } catch (e: unknown) {
    yield { type: "error", text: `Failed to spawn claude: ${e instanceof Error ? e.message : String(e)}` };
    return;
  }

  // Handle abort
  if (opts.abortSignal) {
    opts.abortSignal.addEventListener("abort", () => {
      proc.kill("SIGTERM");
    }, { once: true });
  }

  // Parse streaming JSON output line by line
  const rl = readline.createInterface({ input: proc.stdout!, crlfDelay: Infinity });

  let sessionId: string | undefined;
  let fullText = "";

  try {
    for await (const line of rl) {
      if (!line.trim()) continue;

      let event: any;
      try {
        event = JSON.parse(line);
      } catch {
        continue; // skip non-JSON lines
      }

      // Extract session ID
      if (event.session_id) {
        sessionId = event.session_id;
        yield { type: "session", sessionId };
      }

      // Process by message type
      if (event.type === "assistant") {
        const content = event.message?.content || event.content;
        if (Array.isArray(content)) {
          for (const block of content) {
            if (block.type === "text" && block.text) {
              fullText += block.text;
              yield { type: "text", text: block.text };
            } else if (block.type === "tool_use" && block.name) {
              yield { type: "tool_use", name: block.name };
            }
          }
        }
      } else if (event.type === "result") {
        if (event.result && typeof event.result === "string" && !fullText) {
          yield { type: "text", text: event.result };
        }
        if (event.session_id) {
          sessionId = event.session_id;
        }
      }
    }
  } catch (e: unknown) {
    if (opts.abortSignal?.aborted) {
      yield { type: "done", sessionId };
      return;
    }
    yield { type: "error", text: e instanceof Error ? e.message : String(e) };
  }

  // Clean up temp config
  try { fs.unlinkSync(mcpConfigPath); } catch { /* ignore */ }

  // Wait for process to exit
  await new Promise<void>((resolve) => {
    proc.on("close", () => resolve());
    // Timeout: if process doesn't exit in 5s, kill it
    setTimeout(() => { proc.kill("SIGKILL"); resolve(); }, 5000);
  });

  yield { type: "done", sessionId };
}
