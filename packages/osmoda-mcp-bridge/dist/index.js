#!/usr/bin/env node
/**
 * osModa MCP Bridge — exposes all 90 osModa tools as an MCP server over stdio.
 *
 * Usage:
 *   node index.js                     # MCP server on stdin/stdout
 *   node index.js --list-tools        # Print tool names and exit
 *
 * Hermes config (~/.hermes/config.yaml):
 *   mcp_servers:
 *     osmoda:
 *       command: "node"
 *       args: ["/opt/osmoda/packages/osmoda-mcp-bridge/index.js"]
 *
 * Any MCP client can connect to this server and use all osModa daemon tools.
 */
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { CallToolRequestSchema, ListToolsRequestSchema, } from "@modelcontextprotocol/sdk/types.js";
import { getAllTools } from "./tools.js";
import { teachd } from "./daemon-clients.js";
// ---------------------------------------------------------------------------
// Automatic tool execution logging for skill auto-teaching (teachd)
// ---------------------------------------------------------------------------
const TEACHD_SKIP_TOOLS = new Set([
    "teach_observe_action", "teach_skill_execution", "teach_skill_detect",
    "teach_skill_verify", "teach_status", "teach_observations", "teach_patterns",
    "teach_knowledge", "teach_knowledge_create", "teach_context",
    "teach_optimize_suggest", "teach_optimize_apply", "teach_skill_candidates",
    "teach_skill_generate", "teach_skill_promote",
]);
const SESSION_TIMEOUT_MS = 30 * 60 * 1000;
let currentSessionId = `session-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
let lastToolTime = Date.now();
function getSessionId() {
    const now = Date.now();
    if (now - lastToolTime > SESSION_TIMEOUT_MS) {
        currentSessionId = `session-${now}-${Math.random().toString(36).slice(2, 8)}`;
    }
    lastToolTime = now;
    return currentSessionId;
}
function logToolExecution(toolName, params, output, success) {
    if (TEACHD_SKIP_TOOLS.has(toolName))
        return;
    teachd("POST", "/observe/action", {
        tool: toolName,
        params: typeof params === "object" && Object.keys(params).length > 0 ? params : {},
        result_summary: typeof output === "string" ? output.substring(0, 200) : "",
        session_id: getSessionId(),
        success,
    }).catch(() => { }); // best-effort — teachd being down never breaks tools
}
// Load all 90 tools
const tools = getAllTools();
const toolMap = new Map();
for (const t of tools)
    toolMap.set(t.name, t);
// --list-tools flag: print tool names and exit
if (process.argv.includes("--list-tools")) {
    console.log(`osModa MCP Bridge — ${tools.length} tools available:\n`);
    for (const t of tools)
        console.log(`  ${t.name.padEnd(28)} ${t.description.slice(0, 70)}`);
    process.exit(0);
}
// Create MCP server
const server = new Server({ name: "osmoda-mcp-bridge", version: "0.1.0" }, { capabilities: { tools: {} } });
// Handle tools/list
server.setRequestHandler(ListToolsRequestSchema, async () => ({
    tools: tools.map(t => ({
        name: t.name,
        description: t.description,
        inputSchema: t.inputSchema,
    })),
}));
// Handle tools/call
server.setRequestHandler(CallToolRequestSchema, async (request) => {
    const { name, arguments: args } = request.params;
    const tool = toolMap.get(name);
    if (!tool) {
        return {
            content: [{ type: "text", text: JSON.stringify({ error: `Unknown tool: ${name}` }) }],
            isError: true,
        };
    }
    try {
        const result = await tool.handler(args || {});
        // Detect errors in result for accurate logging
        let success = true;
        try {
            const parsed = JSON.parse(result);
            if (parsed && typeof parsed === "object" && "error" in parsed)
                success = false;
        }
        catch { /* not JSON, assume success */ }
        logToolExecution(name, args || {}, result, success);
        return {
            content: [{ type: "text", text: result }],
        };
    }
    catch (e) {
        logToolExecution(name, args || {}, e.message || "exception", false);
        return {
            content: [{ type: "text", text: JSON.stringify({ error: e.message }) }],
            isError: true,
        };
    }
});
// Connect to stdio transport and run
async function main() {
    const transport = new StdioServerTransport();
    await server.connect(transport);
    // Server runs until stdin closes
}
main().catch((e) => {
    console.error("MCP Bridge fatal:", e);
    process.exit(1);
});
