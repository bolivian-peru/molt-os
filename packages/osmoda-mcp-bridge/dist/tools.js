/**
 * All 90 osModa tool definitions for MCP.
 * Each tool maps to a daemon Unix socket call or local shell execution.
 *
 * Tool categories:
 *   agentd (6), system (4), systemd (2), network (1), wallet (7), switch (5),
 *   watcher (2), routine (3), identity (1), receipt (3), voice (5), backup (2),
 *   mesh (11), mcp (4), safety (4), teach (14), approval (4), sandbox (2),
 *   fleet (4), app (6), wallet_tx (1)
 */
import * as fs from "node:fs";
import * as path from "node:path";
import * as child_process from "node:child_process";
import { agentd, keyd, watch, routines, mesh, mcpd, teachd, voice, runShell, runExec } from "./daemon-clients.js";
// Rate limiting for shell_exec
const shellTimestamps = [];
const SHELL_RATE_LIMIT = 30;
const SHELL_WINDOW = 60_000;
// Dangerous command patterns
const DANGEROUS = [
    "rm -rf /", "rm -rf /*", "dd if=", "mkfs.", "> /dev/sd", "chmod -R 777 /",
    ":(){:|:&};:", "mv /* ", "wget|sh", "curl|sh", "python -c.*import os",
    "shutdown", "reboot", "init 0", "halt", "poweroff", "nixos-rebuild",
    "systemctl disable sshd",
];
function normalizeCmd(c) { return c.replace(/\s+/g, " ").trim().toLowerCase(); }
// App management helpers
const APP_REGISTRY = "/var/lib/osmoda/apps/registry.json";
const APP_PREFIX = "osmoda-app-";
function loadApps() {
    try {
        return JSON.parse(fs.readFileSync(APP_REGISTRY, "utf8")).apps || {};
    }
    catch {
        return {};
    }
}
function saveApps(apps) {
    const dir = path.dirname(APP_REGISTRY);
    if (!fs.existsSync(dir))
        fs.mkdirSync(dir, { recursive: true });
    const tmp = APP_REGISTRY + ".tmp";
    fs.writeFileSync(tmp, JSON.stringify({ apps }, null, 2));
    fs.renameSync(tmp, APP_REGISTRY);
}
// Build tool list
export function getAllTools() {
    return [
        // ═══════════════════════════════════════════════════════════════
        // AGENTD (6 tools)
        // ═══════════════════════════════════════════════════════════════
        {
            name: "system_health",
            description: "Get system health: CPU, RAM, disk, load average, uptime, hostname from agentd.",
            inputSchema: { type: "object", properties: {}, required: [] },
            handler: async () => agentd("GET", "/health"),
        },
        {
            name: "system_query",
            description: "Query system state: processes, disk, hostname, uptime. Returns structured JSON.",
            inputSchema: { type: "object", properties: { query: { type: "string", description: "processes, disk, hostname, uptime" }, args: { type: "object", description: "Optional args e.g. { sort: cpu, limit: 10 }" } }, required: ["query"] },
            handler: async (p) => agentd("POST", "/system/query", { query: p.query, args: p.args || {} }),
        },
        {
            name: "system_discover",
            description: "Discover all running services: listening ports, systemd units, process info.",
            inputSchema: { type: "object", properties: {}, required: [] },
            handler: async () => agentd("GET", "/system/discover"),
        },
        {
            name: "event_log",
            description: "Query the append-only hash-chained audit ledger. Filter by type, actor, limit.",
            inputSchema: { type: "object", properties: { type: { type: "string" }, actor: { type: "string" }, limit: { type: "number" } } },
            handler: async (p) => {
                const qs = new URLSearchParams();
                if (p.type)
                    qs.set("type", String(p.type));
                if (p.actor)
                    qs.set("actor", String(p.actor));
                if (p.limit)
                    qs.set("limit", String(p.limit));
                return agentd("GET", `/events/log?${qs}`);
            },
        },
        {
            name: "memory_store",
            description: "Store a memory: summary, detail, category, tags. Creates a persistent, searchable entry.",
            inputSchema: { type: "object", properties: { summary: { type: "string" }, detail: { type: "string" }, category: { type: "string" }, tags: { type: "array", items: { type: "string" } } }, required: ["summary"] },
            handler: async (p) => agentd("POST", "/memory/store", p),
        },
        {
            name: "memory_recall",
            description: "Search memory by query. Returns relevant entries ranked by BM25.",
            inputSchema: { type: "object", properties: { query: { type: "string" }, max_results: { type: "number" }, timeframe: { type: "string" } }, required: ["query"] },
            handler: async (p) => agentd("POST", "/memory/recall", p),
        },
        // ═══════════════════════════════════════════════════════════════
        // SYSTEM (4 tools) — shell, files, directories
        // ═══════════════════════════════════════════════════════════════
        {
            name: "shell_exec",
            description: "Execute a shell command with root access. Rate-limited to 30/min. Dangerous commands require approval.",
            inputSchema: { type: "object", properties: { command: { type: "string", description: "Shell command" }, timeout: { type: "number", description: "Timeout ms (default 30000, max 120000)" } }, required: ["command"] },
            handler: async (p) => {
                const cmd = String(p.command);
                const timeout = Math.min(Number(p.timeout) || 30000, 120000);
                // Rate limit
                const now = Date.now();
                shellTimestamps.push(now);
                while (shellTimestamps.length > 0 && shellTimestamps[0] < now - SHELL_WINDOW)
                    shellTimestamps.shift();
                if (shellTimestamps.length > SHELL_RATE_LIMIT)
                    return JSON.stringify({ error: "Rate limit: max 30 shell_exec/min" });
                // Dangerous command check
                const norm = normalizeCmd(cmd);
                if (DANGEROUS.some(d => norm.includes(d)))
                    return JSON.stringify({ error: `Blocked: dangerous command pattern detected in '${cmd.slice(0, 80)}'` });
                // Execute
                const result = runShell(cmd, timeout);
                // Audit log (fire-and-forget)
                agentd("POST", "/memory/ingest", { event: { category: "system", subcategory: "shell_exec", actor: "mcp.agent", summary: "Shell: " + cmd.slice(0, 100), metadata: { command: cmd } } }).catch(() => { });
                return result;
            },
        },
        {
            name: "file_read",
            description: "Read any file. Returns content (capped at maxLines).",
            inputSchema: { type: "object", properties: { path: { type: "string", description: "Absolute path" }, maxLines: { type: "number", description: "Max lines (default 500)" } }, required: ["path"] },
            handler: async (p) => {
                const fp = String(p.path);
                const maxLines = Number(p.maxLines) || 500;
                try {
                    const content = fs.readFileSync(fp, "utf-8");
                    const lines = content.split("\n");
                    return lines.length > maxLines ? lines.slice(0, maxLines).join("\n") + `\n...(truncated ${lines.length - maxLines} lines)` : content;
                }
                catch (e) {
                    return JSON.stringify({ error: e.message });
                }
            },
        },
        {
            name: "file_write",
            description: "Write content to a file. Creates parent directories. Full filesystem access.",
            inputSchema: { type: "object", properties: { path: { type: "string" }, content: { type: "string" }, append: { type: "boolean", description: "Append instead of overwrite" } }, required: ["path", "content"] },
            handler: async (p) => {
                const fp = String(p.path);
                try {
                    fs.mkdirSync(path.dirname(fp), { recursive: true });
                    if (p.append)
                        fs.appendFileSync(fp, String(p.content));
                    else
                        fs.writeFileSync(fp, String(p.content));
                    agentd("POST", "/memory/ingest", { event: { category: "system", subcategory: "file_write", actor: "mcp.agent", summary: `Write: ${fp}`, metadata: { path: fp, size: String(p.content).length } } }).catch(() => { });
                    return JSON.stringify({ written: fp, bytes: String(p.content).length });
                }
                catch (e) {
                    return JSON.stringify({ error: e.message });
                }
            },
        },
        {
            name: "directory_list",
            description: "List directory contents with file sizes and types.",
            inputSchema: { type: "object", properties: { path: { type: "string", description: "Directory path" }, recursive: { type: "boolean" } }, required: ["path"] },
            handler: async (p) => {
                const dp = String(p.path);
                try {
                    const entries = fs.readdirSync(dp, { withFileTypes: true }).map(e => {
                        try {
                            const s = fs.statSync(path.join(dp, e.name));
                            return { name: e.name, type: e.isDirectory() ? "dir" : "file", size: s.size, mtime: s.mtime.toISOString() };
                        }
                        catch {
                            return { name: e.name, type: "unknown", size: 0 };
                        }
                    });
                    return JSON.stringify({ path: dp, entries });
                }
                catch (e) {
                    return JSON.stringify({ error: e.message });
                }
            },
        },
        // ═══════════════════════════════════════════════════════════════
        // SYSTEMD (2) + NETWORK (1)
        // ═══════════════════════════════════════════════════════════════
        {
            name: "service_status",
            description: "Get systemd service status: active state, PID, memory, CPU, recent logs.",
            inputSchema: { type: "object", properties: { unit: { type: "string", description: "Service unit name e.g. nginx.service" } }, required: ["unit"] },
            handler: async (p) => runShell(`systemctl status ${String(p.unit).replace(/[^a-zA-Z0-9@._-]/g, "")} 2>&1 | head -30`),
        },
        {
            name: "journal_logs",
            description: "Read systemd journal logs for a unit.",
            inputSchema: { type: "object", properties: { unit: { type: "string" }, lines: { type: "number", description: "Number of lines (default 50)" }, priority: { type: "string", description: "emerg|alert|crit|err|warning|notice|info|debug" }, since: { type: "string", description: "e.g. '1 hour ago'" } }, required: ["unit"] },
            handler: async (p) => {
                const unit = String(p.unit).replace(/[^a-zA-Z0-9@._-]/g, "");
                const lines = Math.min(Number(p.lines) || 50, 500);
                const args = [`-u`, unit, `-n`, String(lines), `--no-pager`];
                if (p.priority)
                    args.push(`-p`, String(p.priority));
                if (p.since)
                    args.push(`--since`, String(p.since));
                return runExec("journalctl", args);
            },
        },
        {
            name: "network_info",
            description: "Network interfaces, IP addresses, listening ports, DNS, routing table.",
            inputSchema: { type: "object", properties: { detail: { type: "string", description: "interfaces|ports|dns|routes|all (default: all)" } } },
            handler: async (p) => {
                const d = String(p.detail || "all");
                const parts = [];
                if (d === "all" || d === "interfaces")
                    parts.push("=== Interfaces ===\n" + runShell("ip -br addr"));
                if (d === "all" || d === "ports")
                    parts.push("=== Listening Ports ===\n" + runShell("ss -tlnp"));
                if (d === "all" || d === "dns")
                    parts.push("=== DNS ===\n" + runShell("cat /etc/resolv.conf 2>/dev/null"));
                if (d === "all" || d === "routes")
                    parts.push("=== Routes ===\n" + runShell("ip route"));
                return parts.join("\n\n");
            },
        },
        // ═══════════════════════════════════════════════════════════════
        // WALLET (7 tools via keyd)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("wallet_create", "Create crypto wallet (ETH or SOL)", keyd, "POST", "/wallet/create", { chain: { type: "string", description: "ethereum|solana" }, label: { type: "string" } }, ["chain"]),
        ...daemonProxy("wallet_list", "List all wallets", keyd, "GET", "/wallet/list", {}),
        ...daemonProxy("wallet_sign", "Sign a payload with wallet key", keyd, "POST", "/wallet/sign", { wallet_id: { type: "string" }, payload: { type: "string", description: "Hex-encoded payload" } }, ["wallet_id", "payload"]),
        ...daemonProxy("wallet_send", "Build a signed transfer transaction", keyd, "POST", "/wallet/send", { wallet_id: { type: "string" }, to: { type: "string" }, amount: { type: "string" } }, ["wallet_id", "to", "amount"]),
        ...daemonProxy("wallet_delete", "Delete a wallet", keyd, "POST", "/wallet/delete", { wallet_id: { type: "string" } }, ["wallet_id"]),
        ...daemonProxy("wallet_receipt", "Get transaction receipt from ledger", keyd, "GET", "/wallet/receipt", { wallet_id: { type: "string" }, limit: { type: "number" } }),
        ...daemonProxy("wallet_build_tx", "Build a custom transaction", keyd, "POST", "/wallet/build_tx", { wallet_id: { type: "string" }, chain: { type: "string" }, tx_type: { type: "string" }, to: { type: "string" }, amount: { type: "string" }, chain_params: { type: "object" } }, ["wallet_id", "chain", "to", "amount"]),
        // ═══════════════════════════════════════════════════════════════
        // SAFESWITCH (5 via watch)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("safe_switch_begin", "Begin a SafeSwitch deploy transaction with health checks", watch, "POST", "/switch/begin", { plan: { type: "string" }, ttl_secs: { type: "number" }, health_checks: { type: "array", items: { type: "object" } } }, ["plan"]),
        ...daemonProxy("safe_switch_list", "List all SafeSwitch sessions", watch, "GET", "/switch/list", {}),
        ...daemonProxy("safe_switch_status", "Get SafeSwitch session status", watch, "GET", "/switch/status", { id: { type: "string" } }, ["id"], (p) => `/switch/status/${p.id}`),
        ...daemonProxy("safe_switch_commit", "Commit a SafeSwitch session", watch, "POST", "/switch/commit", { id: { type: "string" } }, ["id"], (p) => `/switch/commit/${p.id}`),
        ...daemonProxy("safe_switch_rollback", "Rollback a SafeSwitch session", watch, "POST", "/switch/rollback", { id: { type: "string" } }, ["id"], (p) => `/switch/rollback/${p.id}`),
        // ═══════════════════════════════════════════════════════════════
        // WATCHERS (2 via watch)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("watcher_add", "Add a health watcher", watch, "POST", "/watcher/add", { name: { type: "string" }, check: { type: "object" }, interval_secs: { type: "number" }, actions: { type: "array", items: { type: "object" } } }, ["name", "check"]),
        ...daemonProxy("watcher_list", "List all health watchers", watch, "GET", "/watcher/list", {}),
        // ═══════════════════════════════════════════════════════════════
        // ROUTINES (3 via routines)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("routine_add", "Add a background automation routine", routines, "POST", "/routine/add", { name: { type: "string" }, trigger: { type: "object" }, action: { type: "object" } }, ["name", "trigger", "action"]),
        ...daemonProxy("routine_list", "List all routines", routines, "GET", "/routine/list", {}),
        ...daemonProxy("routine_trigger", "Manually trigger a routine", routines, "POST", "/routine/trigger", { id: { type: "string" } }, ["id"], (p) => `/routine/trigger/${p.id}`),
        // ═══════════════════════════════════════════════════════════════
        // IDENTITY + RECEIPTS (4 via agentd)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("agent_card", "Get the agent's EIP-8004 Agent Card", agentd, "GET", "/agent/card", {}),
        ...daemonProxy("receipt_list", "List audit receipts", agentd, "GET", "/receipts", { type: { type: "string" }, since: { type: "string" }, limit: { type: "number" } }),
        ...daemonProxy("incident_create", "Create an incident workspace", agentd, "POST", "/incident/create", { name: { type: "string" } }, ["name"]),
        ...daemonProxy("incident_step", "Add a step to an incident", agentd, "POST", "/incident/step", { id: { type: "string" }, action: { type: "string" }, result: { type: "string" } }, ["id", "action"], (p) => `/incident/${p.id}/step`),
        // ═══════════════════════════════════════════════════════════════
        // VOICE (5 via voice daemon)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("voice_status", "Voice pipeline status", voice, "GET", "/voice/status", {}),
        ...daemonProxy("voice_speak", "Text-to-speech (local piper)", voice, "POST", "/voice/speak", { text: { type: "string" } }, ["text"]),
        ...daemonProxy("voice_transcribe", "Speech-to-text (local whisper)", voice, "POST", "/voice/transcribe", { audio_path: { type: "string" } }, ["audio_path"]),
        ...daemonProxy("voice_record", "Record audio from microphone", voice, "POST", "/voice/record", { duration_secs: { type: "number" }, transcribe: { type: "boolean" } }),
        ...daemonProxy("voice_listen", "Toggle continuous voice listening", voice, "POST", "/voice/listen", { enabled: { type: "boolean" } }, ["enabled"]),
        // ═══════════════════════════════════════════════════════════════
        // BACKUP (2 via agentd)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("backup_create", "Create a backup of osModa state", agentd, "POST", "/backup/create", {}),
        ...daemonProxy("backup_list", "List available backups", agentd, "GET", "/backup/list", {}),
        // ═══════════════════════════════════════════════════════════════
        // MESH (11 via mesh daemon)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("mesh_identity", "Get mesh network identity", mesh, "GET", "/identity", {}),
        ...daemonProxy("mesh_invite_create", "Create a mesh invite code", mesh, "POST", "/invite/create", { ttl_secs: { type: "number" } }),
        ...daemonProxy("mesh_invite_accept", "Accept a mesh invite", mesh, "POST", "/invite/accept", { invite_code: { type: "string" } }, ["invite_code"]),
        ...daemonProxy("mesh_peers", "List mesh peers", mesh, "GET", "/peers", {}),
        ...daemonProxy("mesh_peer_send", "Send a message to a mesh peer", mesh, "POST", "/peer/send", { id: { type: "string" }, message: { type: "object" } }, ["id", "message"], (p) => `/peer/${p.id}/send`),
        ...daemonProxy("mesh_peer_disconnect", "Disconnect a mesh peer", mesh, "DELETE", "/peer/disconnect", { id: { type: "string" } }, ["id"], (p) => `/peer/${p.id}`),
        ...daemonProxy("mesh_health", "Mesh network health", mesh, "GET", "/health", {}),
        ...daemonProxy("mesh_room_create", "Create a mesh chat room", mesh, "POST", "/room/create", { name: { type: "string" }, peer_ids: { type: "array", items: { type: "string" } } }, ["name"]),
        ...daemonProxy("mesh_room_join", "Join a mesh room", mesh, "POST", "/room/join", { room_id: { type: "string" } }, ["room_id"]),
        ...daemonProxy("mesh_room_send", "Send message to a mesh room", mesh, "POST", "/room/send", { room_id: { type: "string" }, message: { type: "string" } }, ["room_id", "message"]),
        ...daemonProxy("mesh_room_history", "Get mesh room message history", mesh, "GET", "/room/history", { room_id: { type: "string" }, limit: { type: "number" } }, ["room_id"], (p) => `/room/${p.room_id}/history`),
        // ═══════════════════════════════════════════════════════════════
        // MCP SERVER MANAGEMENT (4 via mcpd)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("mcp_servers", "List MCP servers", mcpd, "GET", "/servers", {}),
        ...daemonProxy("mcp_server_start", "Start an MCP server", mcpd, "POST", "/server/start", { name: { type: "string" } }, ["name"], (p) => `/server/${p.name}/start`),
        ...daemonProxy("mcp_server_stop", "Stop an MCP server", mcpd, "POST", "/server/stop", { name: { type: "string" } }, ["name"], (p) => `/server/${p.name}/stop`),
        ...daemonProxy("mcp_server_restart", "Restart an MCP server", mcpd, "POST", "/server/restart", { name: { type: "string" } }, ["name"], (p) => `/server/${p.name}/restart`),
        // ═══════════════════════════════════════════════════════════════
        // SAFETY (4 — direct shell)
        // ═══════════════════════════════════════════════════════════════
        {
            name: "safety_rollback",
            description: "Emergency NixOS rollback to previous generation.",
            inputSchema: { type: "object", properties: {} },
            handler: async () => runShell("nixos-rebuild switch --rollback 2>&1 || echo 'Not on NixOS'"),
        },
        {
            name: "safety_status",
            description: "System safety status: NixOS generation, boot entries, last rollback.",
            inputSchema: { type: "object", properties: {} },
            handler: async () => runShell("echo '=== Current ==='; readlink /nix/var/nix/profiles/system 2>/dev/null || echo 'Not NixOS'; echo '=== Generations ==='; ls -la /nix/var/nix/profiles/system-*-link 2>/dev/null | tail -5 || echo 'N/A'"),
        },
        {
            name: "safety_panic",
            description: "Emergency stop: kill all osModa daemons except agentd.",
            inputSchema: { type: "object", properties: {} },
            handler: async () => {
                const units = ["osmoda-gateway", "osmoda-keyd", "osmoda-watch", "osmoda-routines", "osmoda-mesh", "osmoda-mcpd", "osmoda-teachd", "osmoda-voice", "osmoda-egress"];
                units.forEach(u => { try {
                    child_process.execSync(`systemctl stop ${u} 2>/dev/null`);
                }
                catch { } });
                return JSON.stringify({ stopped: units, agentd: "kept running" });
            },
        },
        {
            name: "safety_restart",
            description: "Restart all osModa daemons.",
            inputSchema: { type: "object", properties: {} },
            handler: async () => runShell("for s in osmoda-agentd osmoda-keyd osmoda-watch osmoda-routines osmoda-mesh osmoda-mcpd osmoda-teachd osmoda-voice osmoda-egress osmoda-gateway; do systemctl restart $s 2>/dev/null; done; echo 'All daemons restarted'"),
        },
        // ═══════════════════════════════════════════════════════════════
        // TEACH (14 via teachd)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("teach_status", "TeachD health and stats", teachd, "GET", "/health", {}),
        ...daemonProxy("teach_observations", "Recent system observations", teachd, "GET", "/observations", { source: { type: "string" }, since: { type: "string" }, limit: { type: "number" } }),
        ...daemonProxy("teach_patterns", "Detected system patterns", teachd, "GET", "/patterns", { type: { type: "string" }, min_confidence: { type: "number" } }),
        ...daemonProxy("teach_knowledge", "Knowledge documents", teachd, "GET", "/knowledge", { category: { type: "string" }, tag: { type: "string" }, limit: { type: "number" } }),
        ...daemonProxy("teach_knowledge_create", "Create a knowledge document", teachd, "POST", "/knowledge/create", { title: { type: "string" }, category: { type: "string" }, content: { type: "string" }, tags: { type: "array", items: { type: "string" } } }, ["title", "content"]),
        ...daemonProxy("teach_context", "Inject relevant knowledge into prompt", teachd, "POST", "/teach", { context: { type: "string" } }, ["context"]),
        ...daemonProxy("teach_optimize_suggest", "Get optimization suggestions", teachd, "POST", "/optimize/suggest", {}),
        ...daemonProxy("teach_optimize_apply", "Apply an optimization via SafeSwitch", teachd, "POST", "/optimize/apply", { id: { type: "string" } }, ["id"], (p) => `/optimize/apply/${p.id}`),
        ...daemonProxy("teach_skill_candidates", "List auto-detected skill candidates", teachd, "GET", "/skills/candidates", { status: { type: "string" }, limit: { type: "number" } }),
        ...daemonProxy("teach_skill_generate", "Generate SKILL.md from a candidate", teachd, "POST", "/skills/generate", { id: { type: "string" } }, ["id"], (p) => `/skills/generate/${p.id}`),
        ...daemonProxy("teach_skill_promote", "Promote a skill to auto-activation", teachd, "POST", "/skills/promote", { id: { type: "string" } }, ["id"], (p) => `/skills/promote/${p.id}`),
        ...daemonProxy("teach_observe_action", "Log an agent action for skill learning", teachd, "POST", "/observe/action", { tool: { type: "string" }, params: { type: "object" }, result_summary: { type: "string" }, context: { type: "string" }, session_id: { type: "string" }, success: { type: "boolean" } }, ["tool"]),
        ...daemonProxy("teach_skill_execution", "Log a skill execution outcome", teachd, "POST", "/skills/execution", { skill_name: { type: "string" }, outcome: { type: "string" }, session_id: { type: "string" }, notes: { type: "string" } }, ["skill_name", "outcome"]),
        ...daemonProxy("teach_skill_detect", "Manually trigger skill sequence detection", teachd, "POST", "/skills/detect", {}),
        // ═══════════════════════════════════════════════════════════════
        // APPROVAL (4 via agentd)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("approval_request", "Request approval for a command", agentd, "POST", "/approval/request", { command: { type: "string" }, reason: { type: "string" } }, ["command"]),
        ...daemonProxy("approval_pending", "List pending approvals", agentd, "GET", "/approval/pending", {}),
        ...daemonProxy("approval_approve", "Approve a pending request", agentd, "POST", "/approval/approve", { id: { type: "string" } }, ["id"], (p) => `/approval/${p.id}/approve`),
        ...daemonProxy("approval_check", "Check approval status", agentd, "GET", "/approval/check", { id: { type: "string" } }, ["id"], (p) => `/approval/${p.id}`),
        // ═══════════════════════════════════════════════════════════════
        // SANDBOX + CAPABILITY (2 via agentd)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("sandbox_exec", "Execute in sandboxed environment", agentd, "POST", "/sandbox/exec", { command: { type: "string" }, ring: { type: "number", description: "1 (approved) or 2 (untrusted)" }, capabilities: { type: "array", items: { type: "string" } }, timeout_secs: { type: "number" } }, ["command"]),
        ...daemonProxy("capability_mint", "Mint a capability token", agentd, "POST", "/capability/mint", { granted_to: { type: "string" }, permissions: { type: "array", items: { type: "string" } }, ttl_secs: { type: "number" } }, ["granted_to", "permissions"]),
        // ═══════════════════════════════════════════════════════════════
        // FLEET (4 via watch)
        // ═══════════════════════════════════════════════════════════════
        ...daemonProxy("fleet_propose", "Propose a fleet-wide SafeSwitch", watch, "POST", "/fleet/propose", { plan: { type: "string" }, peer_ids: { type: "array", items: { type: "string" } }, health_checks: { type: "array", items: { type: "object" } }, quorum_percent: { type: "number" }, timeout_secs: { type: "number" } }, ["plan", "peer_ids"]),
        ...daemonProxy("fleet_status", "Fleet switch status", watch, "GET", "/fleet/status", { id: { type: "string" } }, ["id"], (p) => `/fleet/status/${p.id}`),
        ...daemonProxy("fleet_vote", "Vote on a fleet switch", watch, "POST", "/fleet/vote", { id: { type: "string" }, peer_id: { type: "string" }, approve: { type: "boolean" }, reason: { type: "string" } }, ["id", "peer_id", "approve"], (p) => `/fleet/vote/${p.id}`),
        ...daemonProxy("fleet_rollback", "Rollback a fleet switch", watch, "POST", "/fleet/rollback", { id: { type: "string" } }, ["id"], (p) => `/fleet/rollback/${p.id}`),
        // ═══════════════════════════════════════════════════════════════
        // APP MANAGEMENT (6 — direct systemd-run)
        // ═══════════════════════════════════════════════════════════════
        {
            name: "app_deploy",
            description: "Deploy an app as a managed systemd service.",
            inputSchema: { type: "object", properties: { name: { type: "string" }, command: { type: "string" }, args: { type: "array", items: { type: "string" } }, working_dir: { type: "string" }, env: { type: "object" }, port: { type: "number" }, restart_policy: { type: "string" }, memory_max: { type: "string" }, cpu_quota: { type: "string" } }, required: ["name", "command"] },
            handler: async (p) => {
                const name = String(p.name).replace(/[^a-zA-Z0-9_-]/g, "");
                const unit = APP_PREFIX + name;
                const args = [`--unit`, unit, `--service-type=simple`, `--property=Restart=${p.restart_policy || "on-failure"}`, `--property=StartLimitIntervalSec=0`, `--property=RestartSec=3`];
                if (p.working_dir)
                    args.push(`--working-directory=${p.working_dir}`);
                if (p.memory_max)
                    args.push(`--property=MemoryMax=${p.memory_max}`);
                if (p.cpu_quota)
                    args.push(`--property=CPUQuota=${p.cpu_quota}`);
                if (p.env && typeof p.env === "object") {
                    for (const [k, v] of Object.entries(p.env))
                        args.push(`--setenv=${k}=${v}`);
                }
                args.push("--", String(p.command));
                if (Array.isArray(p.args))
                    p.args.forEach(a => args.push(String(a)));
                const result = runExec("systemd-run", args);
                // Save to registry
                const apps = loadApps();
                apps[name] = { name, command: String(p.command), args: p.args, working_dir: p.working_dir, env: p.env, port: p.port, restart_policy: p.restart_policy || "on-failure", memory_max: p.memory_max, cpu_quota: p.cpu_quota, created_at: new Date().toISOString(), status: "running" };
                saveApps(apps);
                return JSON.stringify({ deployed: name, unit, result: result.trim() });
            },
        },
        {
            name: "app_list",
            description: "List all deployed apps with their status.",
            inputSchema: { type: "object", properties: {} },
            handler: async () => {
                const apps = loadApps();
                const list = Object.values(apps).map((a) => {
                    const unit = APP_PREFIX + a.name;
                    let active = "unknown";
                    try {
                        active = child_process.execSync(`systemctl is-active ${unit} 2>/dev/null`, { encoding: "utf-8" }).trim();
                    }
                    catch {
                        active = "inactive";
                    }
                    return { ...a, active };
                });
                return JSON.stringify(list);
            },
        },
        {
            name: "app_logs",
            description: "Get logs for a deployed app.",
            inputSchema: { type: "object", properties: { name: { type: "string" }, lines: { type: "number" } }, required: ["name"] },
            handler: async (p) => runExec("journalctl", ["-u", APP_PREFIX + String(p.name).replace(/[^a-zA-Z0-9_-]/g, ""), "-n", String(p.lines || 50), "--no-pager"]),
        },
        {
            name: "app_stop",
            description: "Stop a deployed app.",
            inputSchema: { type: "object", properties: { name: { type: "string" } }, required: ["name"] },
            handler: async (p) => {
                const name = String(p.name).replace(/[^a-zA-Z0-9_-]/g, "");
                runShell(`systemctl stop ${APP_PREFIX}${name} 2>/dev/null`);
                const apps = loadApps();
                if (apps[name]) {
                    apps[name].status = "stopped";
                    saveApps(apps);
                }
                return JSON.stringify({ stopped: name });
            },
        },
        {
            name: "app_restart",
            description: "Restart a deployed app.",
            inputSchema: { type: "object", properties: { name: { type: "string" } }, required: ["name"] },
            handler: async (p) => {
                const name = String(p.name).replace(/[^a-zA-Z0-9_-]/g, "");
                const result = runShell(`systemctl restart ${APP_PREFIX}${name} 2>&1`);
                return JSON.stringify({ restarted: name, result: result.trim() });
            },
        },
        {
            name: "app_remove",
            description: "Remove a deployed app (stops and deletes from registry).",
            inputSchema: { type: "object", properties: { name: { type: "string" } }, required: ["name"] },
            handler: async (p) => {
                const name = String(p.name).replace(/[^a-zA-Z0-9_-]/g, "");
                runShell(`systemctl stop ${APP_PREFIX}${name} 2>/dev/null`);
                const apps = loadApps();
                if (apps[name]) {
                    apps[name].status = "removed";
                    saveApps(apps);
                }
                return JSON.stringify({ removed: name });
            },
        },
    ];
}
// ═══════════════════════════════════════════════════════════════
// Helper: generate daemon proxy tool(s) from a spec
// Returns a single-element array for spread into the main array
// ═══════════════════════════════════════════════════════════════
function daemonProxy(name, description, daemon, method, defaultPath, props, required, pathFn) {
    return [{
            name,
            description,
            inputSchema: { type: "object", properties: props, required: required || [] },
            handler: async (params) => {
                try {
                    const p = pathFn ? pathFn(params) : defaultPath;
                    if (method === "GET") {
                        const qs = new URLSearchParams();
                        for (const [k, v] of Object.entries(params)) {
                            if (v !== undefined && v !== null)
                                qs.set(k, String(v));
                        }
                        const query = qs.toString();
                        return await daemon(method, query ? `${p}?${query}` : p);
                    }
                    return await daemon(method, p, params);
                }
                catch (e) {
                    return JSON.stringify({ error: e.message });
                }
            },
        }];
}
