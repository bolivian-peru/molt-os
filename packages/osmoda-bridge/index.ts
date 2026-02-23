/**
 * osModa Bridge Plugin — gives the agent full OS access via agentd + direct shell.
 *
 * Uses the correct OpenClaw registerTool() factory pattern.
 * Each tool's parameters MUST use JSON Schema format with type/properties/required.
 *
 * 54 tools registered:
 *   agentd:  system_health, system_query, system_discover, event_log, memory_store, memory_recall (6)
 *   system:  shell_exec, file_read, file_write, directory_list (4)
 *   systemd: service_status, journal_logs (2)
 *   network: network_info (1)
 *   wallet:  wallet_create, wallet_list, wallet_sign, wallet_send, wallet_delete, wallet_receipt (6, via keyd)
 *   switch:  safe_switch_begin, safe_switch_status, safe_switch_commit, safe_switch_rollback (4, via watch)
 *   watcher: watcher_add, watcher_list (2, via watch)
 *   routine: routine_add, routine_list, routine_trigger (3, via routines)
 *   identity: agent_card (1, via agentd)
 *   receipt: receipt_list, incident_create, incident_step (3, via agentd)
 *   voice:   voice_status, voice_speak, voice_transcribe, voice_record, voice_listen (5, via osmoda-voice)
 *   backup:  backup_create, backup_list (2, via agentd)
 *   mesh:    mesh_identity, mesh_invite_create, mesh_invite_accept, mesh_peers, mesh_peer_send, mesh_peer_disconnect, mesh_health,
 *            mesh_room_create, mesh_room_join, mesh_room_send, mesh_room_history (11, via osmoda-mesh)
 *   safety:  safety_rollback, safety_status, safety_panic, safety_restart (4, direct shell)
 */

import * as http from "node:http";
import * as fs from "node:fs";
import * as path from "node:path";
import * as child_process from "node:child_process";
import { keydRequest } from "./keyd-client";
import { watchRequest } from "./watch-client";
import { routinesRequest } from "./routines-client";
import { VoiceClient } from "./voice-client";
import { meshRequest } from "./mesh-client";

// ---------------------------------------------------------------------------
// agentd Unix socket HTTP client
// ---------------------------------------------------------------------------

const AGENTD_SOCKET = process.env.OSMODA_SOCKET || "/run/osmoda/agentd.sock";

function agentdRequest(method: string, reqPath: string, body?: unknown): Promise<string> {
  return new Promise((resolve, reject) => {
    const payload = body ? JSON.stringify(body) : undefined;
    const req = http.request({
      socketPath: AGENTD_SOCKET, path: reqPath, method,
      headers: {
        "Content-Type": "application/json",
        ...(payload ? { "Content-Length": String(Buffer.byteLength(payload)) } : {}),
      },
    }, (res) => {
      let data = "";
      res.on("data", (c: Buffer) => { data += c.toString(); });
      res.on("end", () => { resolve(data); });
    });
    req.on("error", (e) => reject(e));
    if (payload) req.write(payload);
    req.end();
  });
}

// ---------------------------------------------------------------------------
// Shell helper — returns stdout or error string (never throws)
// ---------------------------------------------------------------------------

function runShell(cmd: string, timeout = 30000): string {
  try {
    return child_process.execSync(cmd, { timeout, maxBuffer: 1024 * 1024, encoding: "utf-8" });
  } catch (e: any) {
    return `[exit ${e.status || 1}] ${e.stderr || e.message}\n${e.stdout || ""}`;
  }
}

/** Run a command without shell interpolation (prevents injection). */
function runExec(binary: string, args: string[], timeout = 30000): string {
  try {
    return child_process.execFileSync(binary, args, { timeout, maxBuffer: 1024 * 1024, encoding: "utf-8" });
  } catch (e: any) {
    return `[exit ${e.status || 1}] ${e.stderr || e.message}\n${e.stdout || ""}`;
  }
}

/** Sanitize a systemd unit name — only allow safe characters. */
function sanitizeUnitName(name: string): string {
  return name.replace(/[^a-zA-Z0-9@._-]/g, '');
}

/** Validate a journalctl priority level. */
function sanitizePriority(p: string): string {
  const valid = ["emerg", "alert", "crit", "err", "warning", "notice", "info", "debug"];
  return valid.includes(p) ? p : "info";
}

/** Allowed base directories for file read/write operations. */
const ALLOWED_FILE_PATHS = ["/var/lib/osmoda/", "/etc/nixos/", "/home/", "/tmp/", "/etc/", "/var/log/"];

/** Validate a file path — reject traversal and restrict to allowed directories. */
function validateFilePath(filePath: string): string | null {
  if (filePath.includes("..")) return "path must not contain '..'";
  const resolved = path.resolve(filePath);
  if (!ALLOWED_FILE_PATHS.some((base) => resolved.startsWith(base))) {
    return `path must be under one of: ${ALLOWED_FILE_PATHS.join(", ")}`;
  }
  return null;
}

/** Known dangerous commands that warrant extra caution. */
const DANGEROUS_COMMANDS = ["rm -rf", "mkfs", "dd if=", "format", "> /dev/", "shred", "wipefs"];

// ---------------------------------------------------------------------------
// Plugin registration
// ---------------------------------------------------------------------------

export default function register(api: any) {

  // --- system_health ---
  api.registerTool(() => ({
    name: "system_health",
    label: "System Health",
    description: "Get system health: CPU, RAM, disk, load average, uptime, hostname from agentd daemon.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("GET", "/health") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["system_health"] });

  // --- system_query ---
  api.registerTool(() => ({
    name: "system_query",
    label: "System Query",
    description: "Query system state via agentd: processes, disk, hostname, uptime. Returns structured JSON.",
    parameters: {
      type: "object",
      properties: {
        query: { type: "string", description: "What to query: processes, disk, hostname, uptime" },
        args: { type: "object", description: "Optional query arguments e.g. { sort: cpu, limit: 10 }" },
      },
      required: ["query"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("POST", "/system/query", { query: params.query, args: params.args || {} }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["system_query"] });

  // --- system_discover ---
  api.registerTool(() => ({
    name: "system_discover",
    label: "Service Discovery",
    description: "Discover all running services: listening ports, systemd units, process info. Detects known service types (nginx, postgres, redis, node, etc.).",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("GET", "/system/discover") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["system_discover"] });

  // --- event_log ---
  api.registerTool(() => ({
    name: "event_log",
    label: "Event Log",
    description: "Query the append-only hash-chained audit ledger. Shows all system events with tamper-proof integrity.",
    parameters: {
      type: "object",
      properties: {
        type: { type: "string", description: "Filter by event type" },
        actor: { type: "string", description: "Filter by actor" },
        limit: { type: "number", description: "Max events to return (default 50)" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const p = new URLSearchParams();
        if (params.type) p.set("type", String(params.type));
        if (params.actor) p.set("actor", String(params.actor));
        p.set("limit", String(params.limit || 50));
        return { output: await agentdRequest("GET", "/events/log?" + p.toString()) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["event_log"] });

  // --- memory_store ---
  api.registerTool(() => ({
    name: "memory_store",
    label: "Memory Store",
    description: "Store important info in the OS long-term memory (hash-chained ledger). Use for conversation summaries, system diagnoses, user patterns.",
    parameters: {
      type: "object",
      properties: {
        summary: { type: "string", description: "One-line summary of what to remember" },
        detail: { type: "string", description: "Full detail text" },
        category: { type: "string", description: "Category: conversation, diagnosis, system.config, user_pattern, error" },
        tags: { type: "string", description: "Comma-separated tags" },
      },
      required: ["summary", "detail", "category"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const tags = typeof params.tags === "string" ? params.tags.split(",").map((t: string) => t.trim()) : [];
        return { output: await agentdRequest("POST", "/memory/store", {
          summary: params.summary, detail: params.detail, category: params.category, tags,
        }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["memory_store"] });

  // --- memory_recall ---
  api.registerTool(() => ({
    name: "memory_recall",
    label: "Memory Recall",
    description: "Search OS memory for past events, diagnoses, configurations, errors, conversation history.",
    parameters: {
      type: "object",
      properties: {
        query: { type: "string", description: "What to search for" },
        max_results: { type: "number", description: "Max results (default 10)" },
        timeframe: { type: "string", description: "How far back: 1h, 24h, 7d, 30d, all" },
      },
      required: ["query"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const cappedParams = { ...params, max_results: Math.min(Number(params.max_results) || 10, 100) };
        return { output: await agentdRequest("POST", "/memory/recall", cappedParams) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["memory_recall"] });

  // --- shell_exec ---
  api.registerTool(() => ({
    name: "shell_exec",
    label: "Shell Execute",
    description: "Execute a shell command with full root access. Use for system administration, diagnostics, package management, service control. Always explain what you are doing and why.",
    parameters: {
      type: "object",
      properties: {
        command: { type: "string", description: "Shell command to execute" },
        timeout: { type: "number", description: "Timeout in ms (default 30000)" },
      },
      required: ["command"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      const cmd = String(params.command);
      const timeout = Math.min(Number(params.timeout) || 30000, 120000); // Cap at 120s
      // Warn on dangerous commands (still execute — agent has full access, but log it)
      const isDangerous = DANGEROUS_COMMANDS.some((d) => cmd.includes(d));
      if (isDangerous) {
        agentdRequest("POST", "/memory/ingest", {
          event: { category: "security", subcategory: "dangerous_command", actor: "openclaw.agent",
            summary: "Dangerous command executed: " + cmd.substring(0, 100),
            metadata: { command: cmd } },
        }).catch(() => {});
      }
      const result = runShell(cmd, timeout);
      agentdRequest("POST", "/memory/ingest", {
        event: { category: "system", subcategory: "shell_exec", actor: "openclaw.agent",
          summary: "Shell: " + cmd.substring(0, 100), detail: "output_length=" + result.length,
          metadata: { command: cmd } },
      }).catch(() => {});
      return { output: result };
    },
  }), { names: ["shell_exec"] });

  // --- file_read ---
  api.registerTool(() => ({
    name: "file_read",
    label: "File Read",
    description: "Read any file on the system. Full filesystem access. Returns file content.",
    parameters: {
      type: "object",
      properties: {
        path: { type: "string", description: "Absolute file path to read" },
        maxLines: { type: "number", description: "Max lines to return (default 500)" },
      },
      required: ["path"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const filePath = String(params.path);
        const pathErr = validateFilePath(filePath);
        if (pathErr) return { output: JSON.stringify({ error: pathErr, path: filePath }) };
        const content = fs.readFileSync(filePath, "utf-8");
        const lines = content.split("\n");
        const limit = Math.min(Number(params.maxLines) || 500, 2000);
        const truncated = lines.length > limit;
        return { output: JSON.stringify({ path: filePath, lines: lines.length, truncated, content: lines.slice(0, limit).join("\n") }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message, path: params.path }) };
      }
    },
  }), { names: ["file_read"] });

  // --- file_write ---
  api.registerTool(() => ({
    name: "file_write",
    label: "File Write",
    description: "Write content to a file. Creates parent directories if needed. Logged to audit ledger.",
    parameters: {
      type: "object",
      properties: {
        path: { type: "string", description: "Absolute file path to write" },
        content: { type: "string", description: "Content to write" },
      },
      required: ["path", "content"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const filePath = String(params.path);
        const pathErr = validateFilePath(filePath);
        if (pathErr) return { output: JSON.stringify({ error: pathErr, path: filePath }) };
        fs.mkdirSync(path.dirname(filePath), { recursive: true });
        // Atomic write: write to .tmp then rename to prevent partial writes
        const tmpPath = filePath + ".osmoda-tmp";
        fs.writeFileSync(tmpPath, String(params.content), "utf-8");
        fs.renameSync(tmpPath, filePath);
        agentdRequest("POST", "/memory/ingest", {
          event: { category: "system", subcategory: "file_write", actor: "openclaw.agent",
            summary: "Wrote: " + filePath, detail: String(params.content).length + " bytes",
            metadata: { path: filePath } },
        }).catch(() => {});
        return { output: JSON.stringify({ ok: true, path: filePath, size: String(params.content).length }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["file_write"] });

  // --- directory_list ---
  api.registerTool(() => ({
    name: "directory_list",
    label: "Directory List",
    description: "List files and directories at a given path. Shows name, type (file/dir/link), and size.",
    parameters: {
      type: "object",
      properties: {
        path: { type: "string", description: "Directory path (default /)" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      const dir = String(params.path || "/");
      try {
        const entries = fs.readdirSync(dir, { withFileTypes: true });
        const result = entries.map((e) => {
          let size = 0;
          try { size = fs.statSync(path.join(dir, e.name)).size; } catch {}
          return { name: e.name, type: e.isDirectory() ? "dir" : e.isSymbolicLink() ? "link" : "file", size };
        });
        return { output: JSON.stringify(result) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["directory_list"] });

  // --- service_status ---
  api.registerTool(() => ({
    name: "service_status",
    label: "Service Status",
    description: "Check systemd service status. Shows active state, logs, memory usage.",
    parameters: {
      type: "object",
      properties: {
        service: { type: "string", description: "Service name e.g. osmoda-agentd, sshd, osmoda-gateway" },
      },
      required: ["service"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      const unit = sanitizeUnitName(String(params.service));
      return { output: runExec("systemctl", ["status", unit, "--no-pager", "-l", "-n", "25"]) };
    },
  }), { names: ["service_status"] });

  // --- journal_logs ---
  api.registerTool(() => ({
    name: "journal_logs",
    label: "Journal Logs",
    description: "Read systemd journal logs. Filter by service unit, priority, and time range.",
    parameters: {
      type: "object",
      properties: {
        unit: { type: "string", description: "Service unit name to filter" },
        lines: { type: "number", description: "Number of log lines (default 50)" },
        priority: { type: "string", description: "Min priority: emerg, alert, crit, err, warning, notice, info, debug" },
        since: { type: "string", description: "Time filter e.g. '1 hour ago', 'today'" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      const args = ["--no-pager"];
      if (params.unit) args.push("-u", sanitizeUnitName(String(params.unit)));
      if (params.priority) args.push("-p", sanitizePriority(String(params.priority)));
      if (params.since) args.push("--since", String(params.since));
      args.push("-n", String(Math.min(Number(params.lines) || 50, 500))); // Cap at 500 lines
      return { output: runExec("journalctl", args) };
    },
  }), { names: ["journal_logs"] });

  // --- network_info ---
  api.registerTool(() => ({
    name: "network_info",
    label: "Network Info",
    description: "Get network information: interfaces, active connections, routes, DNS config, listening ports.",
    parameters: {
      type: "object",
      properties: {
        query: { type: "string", description: "What to show: interfaces, connections, routes, dns, ports (default: interfaces)" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      const cmds: Record<string, string> = {
        interfaces: "ip -j addr show 2>/dev/null || ip addr show",
        connections: "ss -tupn",
        routes: "ip -j route show 2>/dev/null || ip route show",
        dns: "cat /etc/resolv.conf",
        ports: "ss -tlnp",
      };
      const q = String(params.query || "interfaces");
      return { output: runShell(cmds[q] || cmds.interfaces) };
    },
  }), { names: ["network_info"] });

  // =========================================================================
  // Wallet tools (via osmoda-keyd)
  // =========================================================================

  // --- wallet_create ---
  api.registerTool(() => ({
    name: "wallet_create",
    label: "Wallet Create",
    description: "Create a new ETH or SOL wallet. Keys are encrypted at rest and never leave keyd.",
    parameters: {
      type: "object",
      properties: {
        chain: { type: "string", description: "Blockchain: ethereum or solana" },
        label: { type: "string", description: "Human-readable wallet label" },
      },
      required: ["chain", "label"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await keydRequest("POST", "/wallet/create", { chain: params.chain, label: params.label }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["wallet_create"] });

  // --- wallet_list ---
  api.registerTool(() => ({
    name: "wallet_list",
    label: "Wallet List",
    description: "List all wallets with addresses, labels, and chains.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await keydRequest("GET", "/wallet/list") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["wallet_list"] });

  // --- wallet_sign ---
  api.registerTool(() => ({
    name: "wallet_sign",
    label: "Wallet Sign",
    description: "Sign raw bytes with a wallet. Policy-gated (daily limits apply). Payload must be hex-encoded.",
    parameters: {
      type: "object",
      properties: {
        wallet_id: { type: "string", description: "Wallet ID to sign with" },
        payload: { type: "string", description: "Hex-encoded bytes to sign" },
      },
      required: ["wallet_id", "payload"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await keydRequest("POST", "/wallet/sign", { wallet_id: params.wallet_id, payload: params.payload }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["wallet_sign"] });

  // --- wallet_send ---
  api.registerTool(() => ({
    name: "wallet_send",
    label: "Wallet Send",
    description: "Build and sign a transaction (policy-gated). Returns signed tx hex for external broadcast (keyd has no network).",
    parameters: {
      type: "object",
      properties: {
        wallet_id: { type: "string", description: "Wallet ID to send from" },
        to: { type: "string", description: "Destination address" },
        amount: { type: "string", description: "Amount to send (e.g. '0.5')" },
      },
      required: ["wallet_id", "to", "amount"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await keydRequest("POST", "/wallet/send", { wallet_id: params.wallet_id, to: params.to, amount: params.amount }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["wallet_send"] });

  // --- wallet_delete ---
  api.registerTool(() => ({
    name: "wallet_delete",
    label: "Wallet Delete",
    description: "Permanently delete a wallet. Removes the encrypted key file, zeroizes cached key material, and updates the wallet index. This action is irreversible.",
    parameters: {
      type: "object",
      properties: {
        wallet_id: { type: "string", description: "Wallet ID to delete" },
      },
      required: ["wallet_id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await keydRequest("POST", "/wallet/delete", { wallet_id: params.wallet_id }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["wallet_delete"] });

  // --- wallet_receipt ---
  api.registerTool(() => ({
    name: "wallet_receipt",
    label: "Wallet Receipt",
    description: "Get wallet operation receipts from the audit ledger.",
    parameters: {
      type: "object",
      properties: {
        limit: { type: "number", description: "Max receipts to return (default 20)" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const p = new URLSearchParams();
        p.set("type", "wallet.");
        p.set("limit", String(params.limit || 20));
        return { output: await agentdRequest("GET", "/receipts?" + p.toString()) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["wallet_receipt"] });

  // =========================================================================
  // SafeSwitch tools (via osmoda-watch)
  // =========================================================================

  // --- safe_switch_begin ---
  api.registerTool(() => ({
    name: "safe_switch_begin",
    label: "SafeSwitch Begin",
    description: "Start a deploy transaction with health checks and a TTL. If health checks fail or TTL expires, auto-rollback occurs.",
    parameters: {
      type: "object",
      properties: {
        plan: { type: "string", description: "Description of the change being deployed" },
        ttl_secs: { type: "number", description: "Probation period in seconds (default 300)" },
        health_checks: { type: "array", description: "Array of health check objects: {type: 'systemd_unit', unit: 'sshd'}, {type: 'tcp_port', host: '127.0.0.1', port: 22}, {type: 'http_get', url: '...', expect_status: 200}, {type: 'command', cmd: '...', args: [...]}" },
      },
      required: ["plan"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await watchRequest("POST", "/switch/begin", {
          plan: params.plan,
          ttl_secs: params.ttl_secs || 300,
          health_checks: params.health_checks || [],
        }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["safe_switch_begin"] });

  // --- safe_switch_status ---
  api.registerTool(() => ({
    name: "safe_switch_status",
    label: "SafeSwitch Status",
    description: "Check the status of a SafeSwitch deploy session (probation, committed, or rolled back).",
    parameters: {
      type: "object",
      properties: {
        id: { type: "string", description: "Switch session ID" },
      },
      required: ["id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await watchRequest("GET", `/switch/status/${params.id}`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["safe_switch_status"] });

  // --- safe_switch_commit ---
  api.registerTool(() => ({
    name: "safe_switch_commit",
    label: "SafeSwitch Commit",
    description: "Manually commit a SafeSwitch session (mark as permanently applied).",
    parameters: {
      type: "object",
      properties: {
        id: { type: "string", description: "Switch session ID" },
      },
      required: ["id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await watchRequest("POST", `/switch/commit/${params.id}`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["safe_switch_commit"] });

  // --- safe_switch_rollback ---
  api.registerTool(() => ({
    name: "safe_switch_rollback",
    label: "SafeSwitch Rollback",
    description: "Manually rollback a SafeSwitch session to the previous NixOS generation.",
    parameters: {
      type: "object",
      properties: {
        id: { type: "string", description: "Switch session ID" },
      },
      required: ["id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await watchRequest("POST", `/switch/rollback/${params.id}`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["safe_switch_rollback"] });

  // =========================================================================
  // Watcher tools (via osmoda-watch)
  // =========================================================================

  // --- watcher_add ---
  api.registerTool(() => ({
    name: "watcher_add",
    label: "Watcher Add",
    description: "Add an autopilot watcher. Monitors health and automatically takes corrective action (restart service, rollback, notify).",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "Watcher name" },
        check: { type: "object", description: "Health check: {type: 'systemd_unit', unit: 'sshd'} or {type: 'tcp_port', host: '127.0.0.1', port: 22}" },
        interval_secs: { type: "number", description: "Check interval in seconds (default 30)" },
        actions: { type: "array", description: "Escalation actions: [{type: 'restart_service', unit: '...'}, {type: 'rollback_generation'}, {type: 'notify', message: '...'}]" },
      },
      required: ["name", "check", "actions"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await watchRequest("POST", "/watcher/add", params) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["watcher_add"] });

  // --- watcher_list ---
  api.registerTool(() => ({
    name: "watcher_list",
    label: "Watcher List",
    description: "List all active autopilot watchers and their current state (healthy/degraded).",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await watchRequest("GET", "/watcher/list") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["watcher_list"] });

  // =========================================================================
  // Routines tools (via osmoda-routines)
  // =========================================================================

  // --- routine_add ---
  api.registerTool(() => ({
    name: "routine_add",
    label: "Routine Add",
    description: "Schedule a recurring background task (health check, service monitor, log scan, custom command, webhook).",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "Routine name" },
        trigger: { type: "object", description: "Trigger: {type: 'interval', seconds: 300} or {type: 'cron', expression: '*/5 * * * *'}" },
        action: { type: "object", description: "Action: {type: 'health_check'}, {type: 'service_monitor', units: [...]}, {type: 'log_scan', priority: 'err'}, {type: 'command', cmd: '...', args: [...]}" },
      },
      required: ["name", "trigger", "action"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await routinesRequest("POST", "/routine/add", params) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["routine_add"] });

  // --- routine_list ---
  api.registerTool(() => ({
    name: "routine_list",
    label: "Routine List",
    description: "List all scheduled routines with their triggers, last run time, and run count.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await routinesRequest("GET", "/routine/list") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["routine_list"] });

  // --- routine_trigger ---
  api.registerTool(() => ({
    name: "routine_trigger",
    label: "Routine Trigger",
    description: "Manually trigger a routine to run immediately (regardless of its schedule).",
    parameters: {
      type: "object",
      properties: {
        id: { type: "string", description: "Routine ID to trigger" },
      },
      required: ["id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await routinesRequest("POST", `/routine/trigger/${params.id}`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["routine_trigger"] });

  // =========================================================================
  // Identity tools (via agentd)
  // =========================================================================

  // --- agent_card ---
  api.registerTool(() => ({
    name: "agent_card",
    label: "Agent Card",
    description: "Get or generate the EIP-8004 Agent Card — identity + capability discovery for this osModa instance.",
    parameters: {
      type: "object",
      properties: {
        action: { type: "string", description: "Action: 'get' (default) or 'generate'" },
        name: { type: "string", description: "Agent name (for generate)" },
        description: { type: "string", description: "Agent description (for generate)" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        if (params.action === "generate") {
          return { output: await agentdRequest("POST", "/agent/card/generate", {
            name: params.name || "osModa",
            description: params.description || "AI-native OS agent",
            services: [],
          }) };
        }
        return { output: await agentdRequest("GET", "/agent/card") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["agent_card"] });

  // =========================================================================
  // Receipt + Incident tools (via agentd)
  // =========================================================================

  // --- receipt_list ---
  api.registerTool(() => ({
    name: "receipt_list",
    label: "Receipt List",
    description: "Query structured receipts from the audit ledger. Filter by type (wallet.sign, switch.commit, etc.) and time range.",
    parameters: {
      type: "object",
      properties: {
        type: { type: "string", description: "Filter by receipt type prefix" },
        since: { type: "string", description: "ISO timestamp to filter from" },
        limit: { type: "number", description: "Max receipts (default 50)" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const p = new URLSearchParams();
        if (params.type) p.set("type", String(params.type));
        if (params.since) p.set("since", String(params.since));
        p.set("limit", String(params.limit || 50));
        return { output: await agentdRequest("GET", "/receipts?" + p.toString()) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["receipt_list"] });

  // --- incident_create ---
  api.registerTool(() => ({
    name: "incident_create",
    label: "Incident Create",
    description: "Create an incident workspace for structured troubleshooting. Steps are resumable (Shannon pattern).",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "Incident name/description" },
      },
      required: ["name"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("POST", "/incident/create", { name: params.name }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["incident_create"] });

  // --- incident_step ---
  api.registerTool(() => ({
    name: "incident_step",
    label: "Incident Step",
    description: "Add a step to an incident workspace. Each step records an action, result, and optional receipt.",
    parameters: {
      type: "object",
      properties: {
        id: { type: "string", description: "Incident workspace ID" },
        action: { type: "string", description: "What action was taken" },
        result: { type: "string", description: "Result: success or failed" },
        receipt_id: { type: "string", description: "Optional receipt ID linking to a ledger entry" },
      },
      required: ["id", "action", "result"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("POST", `/incident/${params.id}/step`, {
          action: params.action,
          result: params.result,
          receipt_id: params.receipt_id,
        }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["incident_step"] });

  // =========================================================================
  // Voice tools (via osmoda-voice — 100% local, no cloud, no tracking)
  // All STT via whisper.cpp, TTS via piper-tts, audio via PipeWire.
  // =========================================================================

  const voiceClient = new VoiceClient(process.env.OSMODA_VOICE_SOCKET || "/run/osmoda/voice.sock");

  // --- voice_status ---
  api.registerTool(() => ({
    name: "voice_status",
    label: "Voice Status",
    description: "Check voice daemon status: listening state, STT model (whisper.cpp), TTS model (piper). All processing is local — no cloud APIs.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        const status = await voiceClient.status();
        return { output: JSON.stringify(status) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["voice_status"] });

  // --- voice_speak ---
  api.registerTool(() => ({
    name: "voice_speak",
    label: "Voice Speak",
    description: "Speak text aloud using piper-tts (local, open-source TTS). Audio plays via PipeWire. No data leaves the machine.",
    parameters: {
      type: "object",
      properties: {
        text: { type: "string", description: "Text to speak aloud" },
      },
      required: ["text"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const result = await voiceClient.speak(String(params.text));
        return { output: JSON.stringify(result) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["voice_speak"] });

  // --- voice_transcribe ---
  api.registerTool(() => ({
    name: "voice_transcribe",
    label: "Voice Transcribe",
    description: "Transcribe a WAV audio file to text using whisper.cpp (local, open-source STT). No data leaves the machine.",
    parameters: {
      type: "object",
      properties: {
        audio_path: { type: "string", description: "Path to a 16kHz mono WAV file" },
      },
      required: ["audio_path"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const result = await voiceClient.transcribe(String(params.audio_path));
        return { output: JSON.stringify(result) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["voice_transcribe"] });

  // --- voice_record ---
  api.registerTool(() => ({
    name: "voice_record",
    label: "Voice Record",
    description: "Record audio from the microphone via PipeWire and optionally transcribe it. Records locally — no cloud, no tracking. Returns audio path and transcribed text.",
    parameters: {
      type: "object",
      properties: {
        duration_secs: { type: "number", description: "Recording duration in seconds (default 5, max 30)" },
        transcribe: { type: "boolean", description: "Also transcribe the recording (default true)" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const duration = Number(params.duration_secs) || 5;
        const transcribe = params.transcribe !== false;
        const result = await voiceClient.record(duration, transcribe);
        return { output: JSON.stringify(result) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["voice_record"] });

  // --- voice_listen ---
  api.registerTool(() => ({
    name: "voice_listen",
    label: "Voice Listen",
    description: "Enable or disable continuous listening mode. When enabled, the voice daemon actively monitors for speech.",
    parameters: {
      type: "object",
      properties: {
        enabled: { type: "boolean", description: "true to start listening, false to stop" },
      },
      required: ["enabled"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const result = await voiceClient.setListening(Boolean(params.enabled));
        return { output: JSON.stringify(result) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["voice_listen"] });

  // =========================================================================
  // Backup tools (via agentd)
  // =========================================================================

  // --- backup_create ---
  api.registerTool(() => ({
    name: "backup_create",
    label: "Backup Create",
    description: "Create a timestamped backup of all osModa state (ledger, keys, watchers, routines). Stored in /var/backups/osmoda/. Keeps last 7 daily backups.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("POST", "/backup/create") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["backup_create"] });

  // --- backup_list ---
  api.registerTool(() => ({
    name: "backup_list",
    label: "Backup List",
    description: "List available backups with IDs, sizes, and timestamps.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("GET", "/backup/list") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["backup_list"] });

  // =========================================================================
  // Mesh tools (via osmoda-mesh — P2P encrypted agent-to-agent)
  // =========================================================================

  // --- mesh_identity ---
  api.registerTool(() => ({
    name: "mesh_identity",
    label: "Mesh Identity",
    description: "Get this instance's mesh identity: instance_id, public keys, capabilities. Used for P2P agent-to-agent communication.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await meshRequest("GET", "/identity") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mesh_identity"] });

  // --- mesh_invite_create ---
  api.registerTool(() => ({
    name: "mesh_invite_create",
    label: "Mesh Invite Create",
    description: "Create an invite code for another osModa instance to connect. The code is copy-pasteable and expires after a TTL.",
    parameters: {
      type: "object",
      properties: {
        ttl_secs: { type: "number", description: "Invite TTL in seconds (default 3600 = 1 hour)" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await meshRequest("POST", "/invite/create", { ttl_secs: params.ttl_secs }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mesh_invite_create"] });

  // --- mesh_invite_accept ---
  api.registerTool(() => ({
    name: "mesh_invite_accept",
    label: "Mesh Invite Accept",
    description: "Accept an invite code from another osModa instance to establish an encrypted P2P connection.",
    parameters: {
      type: "object",
      properties: {
        invite_code: { type: "string", description: "The invite code received from another instance" },
      },
      required: ["invite_code"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await meshRequest("POST", "/invite/accept", { invite_code: params.invite_code }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mesh_invite_accept"] });

  // --- mesh_peers ---
  api.registerTool(() => ({
    name: "mesh_peers",
    label: "Mesh Peers",
    description: "List all known mesh peers with connection state, last seen time, and endpoints.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await meshRequest("GET", "/peers") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mesh_peers"] });

  // --- mesh_peer_send ---
  api.registerTool(() => ({
    name: "mesh_peer_send",
    label: "Mesh Peer Send",
    description: "Send an encrypted message to a connected mesh peer. Supports chat, alerts, health reports, commands.",
    parameters: {
      type: "object",
      properties: {
        peer_id: { type: "string", description: "Peer instance ID to send to" },
        message: { type: "object", description: "Message object: {type: 'chat', from: 'admin', text: '...'} or {type: 'alert', severity: 'warning', title: '...', detail: '...'}" },
      },
      required: ["peer_id", "message"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await meshRequest("POST", `/peer/${params.peer_id}/send`, { message: params.message }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mesh_peer_send"] });

  // --- mesh_peer_disconnect ---
  api.registerTool(() => ({
    name: "mesh_peer_disconnect",
    label: "Mesh Peer Disconnect",
    description: "Disconnect and remove a mesh peer.",
    parameters: {
      type: "object",
      properties: {
        peer_id: { type: "string", description: "Peer instance ID to disconnect" },
      },
      required: ["peer_id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await meshRequest("DELETE", `/peer/${params.peer_id}`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mesh_peer_disconnect"] });

  // --- mesh_health ---
  api.registerTool(() => ({
    name: "mesh_health",
    label: "Mesh Health",
    description: "Check mesh daemon health: peer count, connected count, identity status.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await meshRequest("GET", "/health") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mesh_health"] });

  // --- mesh_room_create ---
  api.registerTool(() => ({
    name: "mesh_room_create",
    label: "Mesh Room Create",
    description: "Create a named group room for multi-peer communication over the encrypted mesh network.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "Room name" },
      },
      required: ["name"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await meshRequest("POST", "/room/create", { name: params.name }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mesh_room_create"] });

  // --- mesh_room_join ---
  api.registerTool(() => ({
    name: "mesh_room_join",
    label: "Mesh Room Join",
    description: "Add a connected peer to a group room so they receive room messages.",
    parameters: {
      type: "object",
      properties: {
        room_id: { type: "string", description: "Room ID to join" },
        peer_id: { type: "string", description: "Peer instance ID to add to the room" },
      },
      required: ["room_id", "peer_id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await meshRequest("POST", "/room/join", { room_id: params.room_id, peer_id: params.peer_id }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mesh_room_join"] });

  // --- mesh_room_send ---
  api.registerTool(() => ({
    name: "mesh_room_send",
    label: "Mesh Room Send",
    description: "Send a message to all connected members of a group room. Primary tool for multi-agent group communication.",
    parameters: {
      type: "object",
      properties: {
        room_id: { type: "string", description: "Room ID to send to" },
        text: { type: "string", description: "Message text to broadcast to the room" },
      },
      required: ["room_id", "text"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await meshRequest("POST", "/room/send", { room_id: params.room_id, text: params.text }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mesh_room_send"] });

  // --- mesh_room_history ---
  api.registerTool(() => ({
    name: "mesh_room_history",
    label: "Mesh Room History",
    description: "Retrieve recent messages from a group room. Use for context injection before responding in a group conversation.",
    parameters: {
      type: "object",
      properties: {
        room_id: { type: "string", description: "Room ID to fetch history for" },
        limit: { type: "number", description: "Max messages to return (default 50)" },
      },
      required: ["room_id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const p = new URLSearchParams();
        p.set("room_id", String(params.room_id));
        if (params.limit) p.set("limit", String(params.limit));
        return { output: await meshRequest("GET", "/room/history?" + p.toString()) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mesh_room_history"] });

  // =========================================================================
  // Safety commands — bypass AI, direct system action
  // =========================================================================

  // --- safety_rollback ---
  api.registerTool(() => ({
    name: "safety_rollback",
    label: "Emergency Rollback",
    description: "SAFETY COMMAND: Immediately rollback NixOS to the previous generation. Bypasses AI — runs nixos-rebuild --rollback switch directly.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      agentdRequest("POST", "/memory/ingest", {
        event: { category: "safety", subcategory: "rollback", actor: "user.direct", summary: "Emergency rollback triggered" },
      }).catch(() => {});
      return { output: runShell("nixos-rebuild --rollback switch 2>&1", 120000) };
    },
  }), { names: ["safety_rollback"] });

  // --- safety_status ---
  api.registerTool(() => ({
    name: "safety_status",
    label: "System Status",
    description: "SAFETY COMMAND: Raw system health dump. Tries agentd first, falls back to shell if agentd is down.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("GET", "/health") };
      } catch {
        const fallback = [
          "--- agentd unreachable, shell fallback ---",
          runShell("uptime"),
          runShell("free -h"),
          runShell("df -h /"),
          runShell("systemctl is-active osmoda-agentd osmoda-gateway || true"),
        ].join("\n");
        return { output: fallback };
      }
    },
  }), { names: ["safety_status"] });

  // --- safety_panic ---
  api.registerTool(() => ({
    name: "safety_panic",
    label: "Panic Stop",
    description: "SAFETY COMMAND: Stop all osModa services except agentd, then rollback NixOS. Use when the system is in a bad state.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      agentdRequest("POST", "/memory/ingest", {
        event: { category: "safety", subcategory: "panic", actor: "user.direct", summary: "Panic stop triggered" },
      }).catch(() => {});
      const stopResult = runShell("systemctl stop osmoda-gateway osmoda-egress osmoda-keyd osmoda-watch osmoda-routines osmoda-voice osmoda-mesh 2>&1 || true");
      const rollbackResult = runShell("nixos-rebuild --rollback switch 2>&1", 120000);
      return { output: `Services stopped:\n${stopResult}\nRollback:\n${rollbackResult}` };
    },
  }), { names: ["safety_panic"] });

  // --- safety_restart ---
  api.registerTool(() => ({
    name: "safety_restart",
    label: "Restart Gateway",
    description: "SAFETY COMMAND: Restart the OpenClaw gateway service.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      agentdRequest("POST", "/memory/ingest", {
        event: { category: "safety", subcategory: "restart", actor: "user.direct", summary: "Gateway restart triggered" },
      }).catch(() => {});
      return { output: runShell("systemctl restart osmoda-gateway 2>&1") };
    },
  }), { names: ["safety_restart"] });
}
