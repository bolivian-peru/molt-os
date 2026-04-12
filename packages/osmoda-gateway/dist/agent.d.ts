/**
 * Claude Code agent wrapper — spawns claude CLI in headless mode with MCP tools.
 *
 * Uses `claude -p --output-format stream-json --verbose` for real-time streaming.
 * Auth: ANTHROPIC_API_KEY env var (Console key) or CLAUDE_CODE_OAUTH_TOKEN (subscription).
 * Permissions: --allowedTools pre-approves MCP tools (works as root).
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
    type: "text" | "tool_use" | "tool_result" | "done" | "error" | "session" | "thinking";
    text?: string;
    name?: string;
    sessionId?: string;
}
/**
 * Call the Claude Code agent with a user message.
 * Spawns `claude -p --output-format stream-json --verbose` and yields real-time streaming events.
 */
export declare function callAgent(opts: AgentCallOptions): AsyncGenerator<AgentEvent>;
