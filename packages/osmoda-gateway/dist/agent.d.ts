/**
 * Claude Code agent wrapper — spawns claude CLI in headless mode with MCP tools.
 *
 * Uses `claude --print --output-format stream-json` for programmatic access.
 * The CLI connects to the osmoda-mcp-bridge MCP server for all 91 system tools.
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
    type: "text" | "tool_use" | "tool_result" | "done" | "error" | "session";
    text?: string;
    name?: string;
    sessionId?: string;
}
/**
 * Call the Claude Code agent with a user message.
 * Spawns `claude --print --output-format stream-json` and yields streaming events.
 */
export declare function callAgent(opts: AgentCallOptions): AsyncGenerator<AgentEvent>;
