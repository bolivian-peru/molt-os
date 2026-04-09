/**
 * Claude Code agent wrapper — spawns claude CLI in headless mode with MCP tools.
 *
 * Uses `claude -p --output-format text` for non-interactive agent calls.
 * Auth: ANTHROPIC_API_KEY env var (Console API key, sk-ant-api03-...).
 * Permissions: --allowedTools pre-approves MCP tools (works as root, unlike --dangerously-skip-permissions).
 * MCP: osmoda-mcp-bridge provides 91 system management tools over stdio.
 */
import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
const __dirname = path.dirname(fileURLToPath(import.meta.url));
/** Resolve claude binary path */
function findClaude() {
    const candidates = [
        process.env.CLAUDE_PATH,
        path.resolve(__dirname, "../node_modules/.bin/claude"),
        "/usr/local/bin/claude",
        "/run/current-system/sw/bin/claude",
    ].filter(Boolean);
    for (const p of candidates) {
        try {
            fs.accessSync(p, fs.constants.X_OK);
            return p;
        }
        catch { /* next */ }
    }
    return "claude"; // hope it's on PATH
}
/** Build MCP config JSON for Claude Code */
function buildMcpConfig(mcpBridgePath) {
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
// Persistent MCP config file (written once, reused across calls)
let mcpConfigPath = null;
function getMcpConfigPath(mcpBridgePath) {
    if (!mcpConfigPath) {
        mcpConfigPath = "/var/lib/osmoda/config/mcp-bridge.json";
        try {
            fs.mkdirSync(path.dirname(mcpConfigPath), { recursive: true });
            fs.writeFileSync(mcpConfigPath, JSON.stringify(buildMcpConfig(mcpBridgePath), null, 2));
        }
        catch {
            // Fallback: unique temp file per process
            mcpConfigPath = `/tmp/osmoda-mcp-${process.pid}.json`;
            fs.writeFileSync(mcpConfigPath, JSON.stringify(buildMcpConfig(mcpBridgePath), null, 2));
        }
    }
    return mcpConfigPath;
}
/**
 * Call the Claude Code agent with a user message.
 * Spawns `claude -p` and yields events parsed from text output.
 */
export async function* callAgent(opts) {
    const claude = findClaude();
    const cwd = opts.cwd || "/root";
    const configPath = getMcpConfigPath(opts.mcpBridgePath);
    // Auth strategy:
    // - If ANTHROPIC_API_KEY is set (Console key): use --bare mode (fastest, no OAuth)
    // - If CLAUDE_CODE_OAUTH_TOKEN is set: normal mode with OAuth (subscription credits)
    // - If neither: normal mode, Claude Code uses its own stored credentials
    const hasApiKey = !!process.env.ANTHROPIC_API_KEY;
    const args = [
        "-p", // Print mode (non-interactive)
        "--output-format", "text", // Simple text output (most reliable)
        "--model", opts.model, // Model selection (v2.1.97+)
        "--system-prompt", opts.systemPrompt.slice(0, 10000), // System prompt (truncate if huge)
        "--mcp-config", configPath, // MCP server config
        "--allowedTools", "mcp__osmoda__*", // Pre-approve all osmoda MCP tools (works as root!)
        "--max-turns", "10", // Limit agentic loops
    ];
    // --bare disables OAuth (only reads ANTHROPIC_API_KEY). Use it only when API key is set.
    // Without --bare, Claude Code reads OAuth from keychain/credentials (subscription credits).
    if (hasApiKey) {
        args.push("--bare");
    }
    // Resume session if we have one
    if (opts.sessionId) {
        args.push("--resume", opts.sessionId);
    }
    // The prompt goes at the end
    args.push(opts.message);
    let proc;
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
    }
    catch (e) {
        yield { type: "error", text: `Failed to spawn claude: ${e instanceof Error ? e.message : String(e)}` };
        return;
    }
    // Handle abort
    if (opts.abortSignal) {
        opts.abortSignal.addEventListener("abort", () => {
            proc.kill("SIGTERM");
        }, { once: true });
    }
    // Collect stdout (text output)
    let fullText = "";
    const chunks = [];
    proc.stdout?.on("data", (data) => {
        const text = data.toString();
        chunks.push(text);
        fullText += text;
    });
    // Collect stderr for errors
    let stderrText = "";
    proc.stderr?.on("data", (data) => {
        stderrText += data.toString();
    });
    // Wait for process to finish
    const exitCode = await new Promise((resolve) => {
        proc.on("close", (code) => resolve(code ?? 1));
        // Timeout: kill after 120s
        setTimeout(() => {
            proc.kill("SIGKILL");
            resolve(124);
        }, 120000);
    });
    if (opts.abortSignal?.aborted) {
        yield { type: "done" };
        return;
    }
    if (exitCode !== 0 && !fullText.trim()) {
        // Process failed with no output
        const errMsg = stderrText.trim() || `claude exited with code ${exitCode}`;
        yield { type: "error", text: errMsg };
        return;
    }
    // Emit the full response as text
    if (fullText.trim()) {
        yield { type: "text", text: fullText.trim() };
    }
    yield { type: "done" };
}
