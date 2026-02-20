/**
 * AgentOS Bridge Plugin — gives the agent full OS access via agentd + direct shell.
 *
 * Uses the correct OpenClaw registerTool() factory pattern.
 * Each tool's parameters MUST use JSON Schema format with type/properties/required.
 *
 * Tools registered:
 *   agentd:  system_health, system_query, event_log, memory_store, memory_recall
 *   system:  shell_exec, file_read, file_write, directory_list
 *   systemd: service_status, journal_logs
 *   network: network_info
 */

import * as http from "node:http";
import * as fs from "node:fs";
import * as path from "node:path";
import * as child_process from "node:child_process";

// ---------------------------------------------------------------------------
// agentd Unix socket HTTP client
// ---------------------------------------------------------------------------

const AGENTD_SOCKET = process.env.AGENTOS_SOCKET || "/run/agentos/agentd.sock";

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
        return { output: await agentdRequest("POST", "/memory/recall", params) };
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
      const timeout = Number(params.timeout) || 30000;
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
        const content = fs.readFileSync(String(params.path), "utf-8");
        const lines = content.split("\n");
        const limit = Number(params.maxLines) || 500;
        const truncated = lines.length > limit;
        return { output: JSON.stringify({ path: params.path, lines: lines.length, truncated, content: lines.slice(0, limit).join("\n") }) };
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
        fs.mkdirSync(path.dirname(filePath), { recursive: true });
        fs.writeFileSync(filePath, String(params.content), "utf-8");
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
        service: { type: "string", description: "Service name e.g. agentd, sshd, openclaw-gateway" },
      },
      required: ["service"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      return { output: runShell("systemctl status " + String(params.service) + " --no-pager -l 2>&1 | head -25") };
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
      let cmd = "journalctl --no-pager";
      if (params.unit) cmd += " -u " + String(params.unit);
      if (params.priority) cmd += " -p " + String(params.priority);
      if (params.since) cmd += " --since \"" + String(params.since) + "\"";
      cmd += " -n " + String(params.lines || 50);
      return { output: runShell(cmd) };
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
}
