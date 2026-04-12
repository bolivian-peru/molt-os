/**
 * Claude Code agent wrapper — spawns claude CLI in headless mode with MCP tools.
 *
 * Uses `claude -p --output-format stream-json --verbose` for real-time streaming.
 * Auth: ANTHROPIC_API_KEY env var (Console key) or CLAUDE_CODE_OAUTH_TOKEN (subscription).
 * Permissions: --allowedTools pre-approves MCP tools (works as root).
 * MCP: osmoda-mcp-bridge provides 91 system management tools over stdio.
 */

import { spawn, type ChildProcess } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import * as readline from "node:readline";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

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
  type: "text" | "tool_use" | "tool_result" | "done" | "error" | "session" | "thinking";
  text?: string;
  name?: string;
  sessionId?: string;
}

/** Resolve claude binary path */
function findClaude(): string {
  const candidates = [
    process.env.CLAUDE_PATH,
    path.resolve(__dirname, "../node_modules/.bin/claude"),
    "/usr/local/bin/claude",
    "/run/current-system/sw/bin/claude",
  ].filter(Boolean) as string[];

  for (const p of candidates) {
    try {
      fs.accessSync(p, fs.constants.X_OK);
      return p;
    } catch { /* next */ }
  }
  return "claude";
}

/** Build MCP config JSON for Claude Code */
function buildMcpConfig(mcpBridgePath: string): object {
  return {
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
}

// Persistent MCP config file
let mcpConfigPath: string | null = null;
function getMcpConfigPath(mcpBridgePath: string): string {
  if (!mcpConfigPath) {
    mcpConfigPath = "/var/lib/osmoda/config/mcp-bridge.json";
    try {
      fs.mkdirSync(path.dirname(mcpConfigPath), { recursive: true });
      fs.writeFileSync(mcpConfigPath, JSON.stringify(buildMcpConfig(mcpBridgePath), null, 2));
      fs.chmodSync(mcpConfigPath, 0o644);
    } catch {
      mcpConfigPath = `/tmp/osmoda-mcp-${process.pid}.json`;
      fs.writeFileSync(mcpConfigPath, JSON.stringify(buildMcpConfig(mcpBridgePath), null, 2));
    }
  }
  return mcpConfigPath;
}

/**
 * Call the Claude Code agent with a user message.
 * Spawns `claude -p --output-format stream-json --verbose` and yields real-time streaming events.
 */
export async function* callAgent(opts: AgentCallOptions): AsyncGenerator<AgentEvent> {
  const claude = findClaude();
  const cwd = opts.cwd || "/root";
  const configPath = getMcpConfigPath(opts.mcpBridgePath);

  const hasApiKey = !!process.env.ANTHROPIC_API_KEY;

  const args = [
    "-p",                                 // Print mode (non-interactive)
    "--output-format", "stream-json",     // Stream JSON events in real-time
    "--verbose",                          // Required for stream-json
    "--model", opts.model,                // Model selection
    "--system-prompt", opts.systemPrompt.slice(0, 10000),
    "--mcp-config", configPath,
    "--allowedTools", "Bash,Read,Write,Edit,Glob,Grep,WebFetch,mcp__osmoda__*",
  ];

  if (hasApiKey) {
    args.push("--bare");
  }

  if (opts.sessionId) {
    args.push("--resume", opts.sessionId);
  }

  // -- separates flags from the prompt (prevents prompt being parsed as flag value)
  args.push("--", opts.message);

  let proc: ChildProcess;
  try {
    proc = spawn(claude, args, {
      cwd,
      env: {
        ...process.env,
        HOME: process.env.HOME || "/root",
        NODE_ENV: "production",
      },
      stdio: ["pipe", "pipe", "pipe"],
    });
  } catch (e: unknown) {
    yield { type: "error", text: `Failed to spawn claude: ${e instanceof Error ? e.message : String(e)}` };
    return;
  }

  if (opts.abortSignal) {
    opts.abortSignal.addEventListener("abort", () => {
      proc.kill("SIGTERM");
    }, { once: true });
  }

  // Parse stream-json output line by line for real-time events
  const rl = readline.createInterface({ input: proc.stdout!, crlfDelay: Infinity });

  let sessionId: string | undefined;
  let lastTextLen = 0;

  try {
    for await (const line of rl) {
      if (!line.trim()) continue;

      let event: any;
      try {
        event = JSON.parse(line);
      } catch {
        continue; // skip non-JSON lines (warnings, etc.)
      }

      // Capture session ID
      if (event.session_id) {
        sessionId = event.session_id;
        yield { type: "session", sessionId };
      }

      // Process by event type
      const msg = event.message || event;
      const msgType = msg.type || event.type;

      if (msgType === "assistant") {
        const content = msg.content || [];
        if (Array.isArray(content)) {
          for (const block of content) {
            if (block.type === "text" && block.text) {
              // Stream text incrementally (delta from last seen)
              const newText = block.text;
              if (newText.length > lastTextLen) {
                const delta = newText.slice(lastTextLen);
                lastTextLen = newText.length;
                yield { type: "text", text: delta };
              }
            } else if (block.type === "tool_use" && block.name) {
              yield { type: "tool_use", name: block.name };
            }
          }
        }
      } else if (msgType === "tool_result" || msgType === "tool_output") {
        // Tool execution completed
        yield { type: "tool_result" };
      } else if (msgType === "result") {
        // Final result
        if (msg.result && typeof msg.result === "string" && lastTextLen === 0) {
          yield { type: "text", text: msg.result };
        }
        if (msg.session_id) sessionId = msg.session_id;
      } else if (msgType === "system" && msg.subtype === "init") {
        // Init event — session started
        if (msg.session_id) {
          sessionId = msg.session_id;
          yield { type: "session", sessionId };
        }
      }
    }
  } catch (e: unknown) {
    if (opts.abortSignal?.aborted) {
      yield { type: "done", sessionId };
      return;
    }
    // Don't yield error for readline close
  }

  // Collect stderr for error reporting
  let stderrText = "";
  proc.stderr?.on("data", (data: Buffer) => {
    stderrText += data.toString();
  });

  // Wait for process to exit
  const exitCode = await new Promise<number>((resolve) => {
    proc.on("close", (code) => resolve(code ?? 1));
    setTimeout(() => {
      proc.kill("SIGKILL");
      resolve(124);
    }, 600000); // 10 minute timeout
  });

  if (exitCode !== 0 && lastTextLen === 0 && !opts.abortSignal?.aborted) {
    const errMsg = stderrText.trim().split("\n").pop() || `claude exited with code ${exitCode}`;
    yield { type: "error", text: errMsg };
  }

  yield { type: "done", sessionId };
}
