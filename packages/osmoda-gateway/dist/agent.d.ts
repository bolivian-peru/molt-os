/**
 * Claude Code agent wrapper — spawns claude CLI in headless mode with MCP tools.
 *
 * Uses `claude -p --output-format text` for non-interactive agent calls.
 * Auth: ANTHROPIC_API_KEY env var (Console API key, sk-ant-api03-...).
 * Permissions: --allowedTools pre-approves MCP tools (works as root, unlike --dangerously-skip-permissions).
 * MCP: osmoda-mcp-bridge provides 91 system management tools over stdio.
 */
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
    type: "text" | "tool_use" | "done" | "error" | "session";
    text?: string;
    name?: string;
    sessionId?: string;
}
/**
 * Call the Claude Code agent with a user message.
 * Spawns `claude -p` and yields events parsed from text output.
 */
export declare function callAgent(opts: AgentCallOptions): AsyncGenerator<AgentEvent>;
