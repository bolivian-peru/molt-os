/**
 * osModa Bridge Plugin — gives the agent full OS access via agentd + direct shell.
 *
 * Uses the correct OpenClaw registerTool() factory pattern.
 * Each tool's parameters MUST use JSON Schema format with type/properties/required.
 *
 * 83 tools registered:
 *   agentd:   system_health, system_query, system_discover, event_log, memory_store, memory_recall (6)
 *   system:   shell_exec, file_read, file_write, directory_list (4)
 *   systemd:  service_status, journal_logs (2)
 *   network:  network_info (1)
 *   wallet:   wallet_create, wallet_list, wallet_sign, wallet_send, wallet_delete, wallet_receipt, wallet_build_tx (7, via keyd)
 *   switch:   safe_switch_begin, safe_switch_status, safe_switch_commit, safe_switch_rollback (4, via watch)
 *   watcher:  watcher_add, watcher_list (2, via watch)
 *   routine:  routine_add, routine_list, routine_trigger (3, via routines)
 *   identity: agent_card (1, via agentd)
 *   receipt:  receipt_list, incident_create, incident_step (3, via agentd)
 *   voice:    voice_status, voice_speak, voice_transcribe, voice_record, voice_listen (5, via osmoda-voice)
 *   backup:   backup_create, backup_list (2, via agentd)
 *   mesh:     mesh_identity, mesh_invite_create, mesh_invite_accept, mesh_peers, mesh_peer_send, mesh_peer_disconnect, mesh_health,
 *             mesh_room_create, mesh_room_join, mesh_room_send, mesh_room_history (11, via osmoda-mesh)
 *   mcp:      mcp_servers, mcp_server_start, mcp_server_stop, mcp_server_restart (4, via osmoda-mcpd)
 *   teach:    teach_status, teach_observations, teach_patterns, teach_knowledge, teach_knowledge_create,
 *             teach_context, teach_optimize_suggest, teach_optimize_apply (8, via osmoda-teachd)
 *   approval: approval_request, approval_pending, approval_approve, approval_check (4, via agentd)
 *   sandbox:  sandbox_exec, capability_mint (2, via agentd)
 *   fleet:    fleet_propose, fleet_status, fleet_vote, fleet_rollback (4, via watch)
 *   app:      app_deploy, app_list, app_logs, app_stop, app_restart, app_remove (6, direct systemd-run)
 *   safety:   safety_rollback, safety_status, safety_panic, safety_restart (4, direct shell)
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
import { mcpdRequest } from "./mcpd-client";
import { teachdRequest } from "./teachd-client";

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

// ---------------------------------------------------------------------------
// App process management — registry + systemd-run helpers
// ---------------------------------------------------------------------------

const APP_REGISTRY_PATH = "/var/lib/osmoda/apps/registry.json";
const APP_UNIT_PREFIX = "osmoda-app-";

interface AppEntry {
  name: string;
  command: string;
  args?: string[];
  working_dir?: string;
  env?: Record<string, string>;
  memory_max?: string;
  cpu_quota?: string;
  restart_policy: string;
  port?: number;
  user?: string;
  created_at: string;
  status: "running" | "stopped" | "removed";
}

interface AppRegistry {
  apps: Record<string, AppEntry>;
}

function loadAppRegistry(): AppRegistry {
  try {
    const data = fs.readFileSync(APP_REGISTRY_PATH, "utf-8");
    return JSON.parse(data) as AppRegistry;
  } catch {
    return { apps: {} };
  }
}

function saveAppRegistry(registry: AppRegistry): void {
  const dir = path.dirname(APP_REGISTRY_PATH);
  fs.mkdirSync(dir, { recursive: true });
  const tmp = APP_REGISTRY_PATH + ".tmp";
  fs.writeFileSync(tmp, JSON.stringify(registry, null, 2), "utf-8");
  fs.renameSync(tmp, APP_REGISTRY_PATH);
}

function getAppUnitName(name: string): string {
  return APP_UNIT_PREFIX + name.replace(/[^a-zA-Z0-9_-]/g, "-");
}

function getAppStatus(unitName: string): Record<string, unknown> {
  try {
    const raw = child_process.execFileSync("systemctl", [
      "show", unitName,
      "--property=ActiveState,MainPID,MemoryCurrent,CPUUsageNSec,ActiveEnterTimestamp",
    ], { timeout: 5000, encoding: "utf-8" });
    const props: Record<string, string> = {};
    for (const line of raw.trim().split("\n")) {
      const eq = line.indexOf("=");
      if (eq > 0) props[line.slice(0, eq)] = line.slice(eq + 1);
    }
    return {
      active_state: props.ActiveState || "unknown",
      pid: parseInt(props.MainPID || "0", 10) || null,
      memory_bytes: props.MemoryCurrent === "[not set]" ? null : parseInt(props.MemoryCurrent || "0", 10) || null,
      cpu_ns: parseInt(props.CPUUsageNSec || "0", 10) || null,
      started_at: props.ActiveEnterTimestamp || null,
    };
  } catch {
    return { active_state: "unknown", pid: null, memory_bytes: null, cpu_ns: null, started_at: null };
  }
}

const VALID_RESTART_POLICIES = ["no", "on-failure", "on-abnormal", "on-watchdog", "on-abort", "always"];

function validateRestartPolicy(policy: string | undefined): string {
  if (!policy) return "on-failure";
  if (VALID_RESTART_POLICIES.includes(policy)) return policy;
  return "on-failure";
}

/** Allowed base directories for file read/write operations. */
const ALLOWED_FILE_PATHS = ["/var/lib/osmoda/", "/etc/nixos/", "/home/", "/tmp/", "/etc/", "/var/log/"];

/** Validate a file path — reject traversal and restrict to allowed directories. */
function validateFilePath(filePath: string): string | null {
  if (filePath.includes("..")) return "path must not contain '..'";
  const resolved = path.resolve(filePath);
  // Check logical path first (fast path)
  if (!ALLOWED_FILE_PATHS.some((base) => resolved.startsWith(base))) {
    return `path must be under one of: ${ALLOWED_FILE_PATHS.join(", ")}`;
  }
  // Resolve symlinks and re-check — prevents symlink escape
  try {
    const real = fs.realpathSync(resolved);
    if (!ALLOWED_FILE_PATHS.some((base) => real.startsWith(base))) {
      return `resolved symlink target must be under one of: ${ALLOWED_FILE_PATHS.join(", ")}`;
    }
  } catch {
    // Path doesn't exist yet (file_write creating new file) — logical check is sufficient
  }
  return null;
}

/**
 * Defense-in-depth command blocklist. NOT a security boundary.
 * Ring 0 agent has full system access by design.
 * This catches obvious destructive patterns from prompt injection.
 * Real protection comes from: NixOS atomicity, approval policies, audit trail.
 */
const DANGEROUS_COMMANDS = [
  "rm -rf", "mkfs", "dd if=", "format", "> /dev/", "shred", "wipefs",
  "chmod 777", "chmod -R", "chown -R /",
  "curl | sh", "wget | sh", "curl | bash", "wget | bash",
  "> /dev/sd", "shutdown", "reboot", "halt", "init 0", "init 6",
  "nix-collect-garbage", "nixos-rebuild",
];

/** Rate limit shell_exec: 30 calls per 60 seconds. */
const shellExecTimestamps: number[] = [];
const SHELL_EXEC_RATE_LIMIT = 30;
const SHELL_EXEC_WINDOW_MS = 60000;

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
      // Rate limit shell_exec
      const now = Date.now();
      shellExecTimestamps.push(now);
      while (shellExecTimestamps.length > 0 && shellExecTimestamps[0] < now - SHELL_EXEC_WINDOW_MS) {
        shellExecTimestamps.shift();
      }
      if (shellExecTimestamps.length > SHELL_EXEC_RATE_LIMIT) {
        return { output: JSON.stringify({ error: "Rate limit exceeded: max 30 shell_exec calls per minute" }) };
      }
      // Block dangerous commands — defense-in-depth, not a security boundary
      const matched = DANGEROUS_COMMANDS.find((d) => cmd.includes(d));
      if (matched) {
        agentdRequest("POST", "/memory/ingest", {
          event: { category: "security", subcategory: "dangerous_command_blocked", actor: "openclaw.agent",
            summary: "Dangerous command blocked: " + cmd.substring(0, 100),
            metadata: { command: cmd, matched_pattern: matched } },
        }).catch(() => {});
        return { output: JSON.stringify({ error: `Command blocked: '${matched}' is not allowed via shell_exec. Use the appropriate agentd API (safe_switch_begin for NixOS changes, or request user approval for destructive operations).` }) };
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
        const stat = fs.statSync(filePath);
        if (stat.size > 10 * 1024 * 1024) {
          return { output: JSON.stringify({ error: "File too large (>10MB)", path: filePath, size: stat.size }) };
        }
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
      const pathErr = validateFilePath(dir);
      if (pathErr) {
        return { output: JSON.stringify({ error: pathErr }) };
      }
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
  // MCP tools (via osmoda-mcpd — MCP server lifecycle management)
  // =========================================================================

  // --- mcp_servers ---
  api.registerTool(() => ({
    name: "mcp_servers",
    label: "MCP Servers",
    description: "List all managed MCP servers with status, pid, restart count, and allowed domains.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await mcpdRequest("GET", "/servers") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mcp_servers"] });

  // --- mcp_server_start ---
  api.registerTool(() => ({
    name: "mcp_server_start",
    label: "MCP Server Start",
    description: "Start a stopped MCP server by name.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "MCP server name to start" },
      },
      required: ["name"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await mcpdRequest("POST", `/server/${params.name}/start`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mcp_server_start"] });

  // --- mcp_server_stop ---
  api.registerTool(() => ({
    name: "mcp_server_stop",
    label: "MCP Server Stop",
    description: "Stop a running MCP server by name.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "MCP server name to stop" },
      },
      required: ["name"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await mcpdRequest("POST", `/server/${params.name}/stop`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mcp_server_stop"] });

  // --- mcp_server_restart ---
  api.registerTool(() => ({
    name: "mcp_server_restart",
    label: "MCP Server Restart",
    description: "Restart an MCP server (stop + start). Increments restart count.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "MCP server name to restart" },
      },
      required: ["name"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await mcpdRequest("POST", `/server/${params.name}/restart`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["mcp_server_restart"] });

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

  // =========================================================================
  // Teaching tools (via osmoda-teachd — system learning & self-optimization)
  // =========================================================================

  // --- teach_status ---
  api.registerTool(() => ({
    name: "teach_status",
    label: "Teach Status",
    description: "Get teachd health: observation, pattern, knowledge, and optimization counts plus loop status.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await teachdRequest("GET", "/health") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["teach_status"] });

  // --- teach_observations ---
  api.registerTool(() => ({
    name: "teach_observations",
    label: "Teach Observations",
    description: "List recent system observations collected by teachd (CPU, memory, service, journal).",
    parameters: {
      type: "object",
      properties: {
        source: { type: "string", description: "Filter by source: cpu, memory, service, journal" },
        since: { type: "string", description: "ISO timestamp to filter observations since" },
        limit: { type: "number", description: "Max results (default 50)" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const qs = new URLSearchParams();
        if (params.source) qs.set("source", String(params.source));
        if (params.since) qs.set("since", String(params.since));
        if (params.limit) qs.set("limit", String(params.limit));
        const path = `/observations${qs.toString() ? "?" + qs.toString() : ""}`;
        return { output: await teachdRequest("GET", path) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["teach_observations"] });

  // --- teach_patterns ---
  api.registerTool(() => ({
    name: "teach_patterns",
    label: "Teach Patterns",
    description: "List detected system patterns (recurring failures, trends, anomalies, correlations).",
    parameters: {
      type: "object",
      properties: {
        type: { type: "string", description: "Filter by pattern type: recurring, trend, anomaly, correlation" },
        min_confidence: { type: "number", description: "Minimum confidence threshold (default 0.5)" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const qs = new URLSearchParams();
        if (params.type) qs.set("type", String(params.type));
        if (params.min_confidence) qs.set("min_confidence", String(params.min_confidence));
        const path = `/patterns${qs.toString() ? "?" + qs.toString() : ""}`;
        return { output: await teachdRequest("GET", path) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["teach_patterns"] });

  // --- teach_knowledge ---
  api.registerTool(() => ({
    name: "teach_knowledge",
    label: "Teach Knowledge",
    description: "List knowledge documents generated from detected patterns or manually created.",
    parameters: {
      type: "object",
      properties: {
        category: { type: "string", description: "Filter by category: performance, reliability, security, configuration" },
        tag: { type: "string", description: "Filter by tag" },
        limit: { type: "number", description: "Max results (default 20)" },
      },
      required: [],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const qs = new URLSearchParams();
        if (params.category) qs.set("category", String(params.category));
        if (params.tag) qs.set("tag", String(params.tag));
        if (params.limit) qs.set("limit", String(params.limit));
        const path = `/knowledge${qs.toString() ? "?" + qs.toString() : ""}`;
        return { output: await teachdRequest("GET", path) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["teach_knowledge"] });

  // --- teach_knowledge_create ---
  api.registerTool(() => ({
    name: "teach_knowledge_create",
    label: "Create Knowledge",
    description: "Manually create a knowledge document to persist system wisdom (e.g. troubleshooting steps, config insights).",
    parameters: {
      type: "object",
      properties: {
        title: { type: "string", description: "Knowledge doc title" },
        category: { type: "string", description: "Category: performance, reliability, security, configuration" },
        content: { type: "string", description: "Markdown content body" },
        tags: { type: "array", items: { type: "string" }, description: "Tags for searchability" },
      },
      required: ["title", "category", "content"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await teachdRequest("POST", "/knowledge/create", params) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["teach_knowledge_create"] });

  // --- teach_context ---
  api.registerTool(() => ({
    name: "teach_context",
    label: "Teach Context",
    description: "Get relevant knowledge documents for a given context string. Returns matching docs and injected token count.",
    parameters: {
      type: "object",
      properties: {
        context: { type: "string", description: "Context string to match against knowledge base" },
      },
      required: ["context"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await teachdRequest("POST", "/teach", { context: params.context }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["teach_context"] });

  // --- teach_optimize_suggest ---
  api.registerTool(() => ({
    name: "teach_optimize_suggest",
    label: "Suggest Optimizations",
    description: "Generate optimization suggestions from unapplied knowledge documents.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await teachdRequest("POST", "/optimize/suggest") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["teach_optimize_suggest"] });

  // --- teach_optimize_apply ---
  api.registerTool(() => ({
    name: "teach_optimize_apply",
    label: "Apply Optimization",
    description: "Apply an approved optimization via SafeSwitch. Must be in 'approved' status first.",
    parameters: {
      type: "object",
      properties: {
        id: { type: "string", description: "Optimization ID to apply" },
      },
      required: ["id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await teachdRequest("POST", `/optimize/apply/${params.id}`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["teach_optimize_apply"] });

  // =========================================================================
  // Approval Gate tools (via agentd — destructive operation approval)
  // =========================================================================

  // --- approval_request ---
  api.registerTool(() => ({
    name: "approval_request",
    label: "Request Approval",
    description: "Request approval for a destructive or sensitive operation. Returns an approval ID to poll. If the command is not classified as destructive, it is auto-approved.",
    parameters: {
      type: "object",
      properties: {
        command: { type: "string", description: "The command or operation identifier (e.g. 'rm -rf /data' or 'nix.rebuild')" },
        reason: { type: "string", description: "Why this operation is needed" },
      },
      required: ["command", "reason"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("POST", "/approval/request", {
          command: params.command, reason: params.reason,
        }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["approval_request"] });

  // --- approval_pending ---
  api.registerTool(() => ({
    name: "approval_pending",
    label: "Pending Approvals",
    description: "List all pending approval requests awaiting user decision.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("GET", "/approval/pending") };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["approval_pending"] });

  // --- approval_approve ---
  api.registerTool(() => ({
    name: "approval_approve",
    label: "Approve Operation",
    description: "Approve a pending destructive operation by its approval ID.",
    parameters: {
      type: "object",
      properties: {
        id: { type: "string", description: "Approval request ID" },
      },
      required: ["id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("POST", `/approval/${params.id}/approve`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["approval_approve"] });

  // --- approval_check ---
  api.registerTool(() => ({
    name: "approval_check",
    label: "Check Approval",
    description: "Check the status of an approval request (pending, approved, denied, expired).",
    parameters: {
      type: "object",
      properties: {
        id: { type: "string", description: "Approval request ID" },
      },
      required: ["id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("GET", `/approval/${params.id}`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["approval_check"] });

  // =========================================================================
  // Sandbox tools (via agentd — Ring 1/Ring 2 isolation)
  // =========================================================================

  // --- sandbox_exec ---
  api.registerTool(() => ({
    name: "sandbox_exec",
    label: "Sandbox Exec",
    description: "Execute a command in a sandboxed environment using bubblewrap (bwrap). Ring 1 = approved apps with declared capabilities. Ring 2 = untrusted, maximum isolation, no network.",
    parameters: {
      type: "object",
      properties: {
        command: { type: "string", description: "Command to execute inside the sandbox" },
        ring: { type: "number", description: "Sandbox ring level: 1 (approved app) or 2 (untrusted). Default: 2" },
        capabilities: {
          type: "array", items: { type: "string" },
          description: "Capability strings (e.g. 'network', 'fs:/var/lib/myapp'). Only applies to Ring 1.",
        },
        timeout_secs: { type: "number", description: "Execution timeout in seconds. Default: 60" },
      },
      required: ["command"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("POST", "/sandbox/exec", {
          command: params.command,
          ring: params.ring || 2,
          capabilities: params.capabilities || [],
          timeout_secs: params.timeout_secs || 60,
        }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["sandbox_exec"] });

  // --- capability_mint ---
  api.registerTool(() => ({
    name: "capability_mint",
    label: "Mint Capability",
    description: "Create a signed capability token granting specific permissions to a tool or app. HMAC-SHA256 signed, time-limited.",
    parameters: {
      type: "object",
      properties: {
        granted_to: { type: "string", description: "Identity receiving the capability (app name or tool ID)" },
        permissions: {
          type: "array", items: { type: "string" },
          description: "Permission strings (e.g. 'network', 'fs:read:/var/lib/data', 'fs:write:/tmp')",
        },
        ttl_secs: { type: "number", description: "Time-to-live in seconds. Default: 3600 (1 hour)" },
      },
      required: ["granted_to", "permissions"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await agentdRequest("POST", "/capability/mint", {
          granted_to: params.granted_to,
          permissions: params.permissions,
          ttl_secs: params.ttl_secs || 3600,
        }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["capability_mint"] });

  // =========================================================================
  // Fleet SafeSwitch tools (via osmoda-watch — coordinated multi-node deploys)
  // =========================================================================

  // --- fleet_propose ---
  api.registerTool(() => ({
    name: "fleet_propose",
    label: "Fleet Propose",
    description: "Initiate a fleet-wide SafeSwitch deployment. Sends a proposal to specified mesh peers for quorum-based voting before execution.",
    parameters: {
      type: "object",
      properties: {
        plan: { type: "string", description: "Description of the deployment plan (e.g. 'upgrade nginx to 1.27')" },
        peer_ids: {
          type: "array", items: { type: "string" },
          description: "List of mesh peer IDs to include in the fleet switch",
        },
        health_checks: {
          type: "array",
          items: {
            type: "object",
            properties: {
              check_type: { type: "string", description: "http_get, tcp_port, systemd_unit, or command" },
              target: { type: "string", description: "URL, port, unit name, or command" },
            },
          },
          description: "Health checks to run after deployment",
        },
        quorum_percent: { type: "number", description: "Approval quorum percentage (default: 51)" },
        timeout_secs: { type: "number", description: "Timeout in seconds (default: 300)" },
      },
      required: ["plan", "peer_ids"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await watchRequest("POST", "/fleet/propose", {
          plan: params.plan,
          peer_ids: params.peer_ids,
          health_checks: params.health_checks,
          quorum_percent: params.quorum_percent,
          timeout_secs: params.timeout_secs,
        }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["fleet_propose"] });

  // --- fleet_status ---
  api.registerTool(() => ({
    name: "fleet_status",
    label: "Fleet Status",
    description: "Check the status of a fleet-wide SafeSwitch: phase, votes, quorum progress, participant health.",
    parameters: {
      type: "object",
      properties: {
        id: { type: "string", description: "Fleet switch ID" },
      },
      required: ["id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await watchRequest("GET", `/fleet/status/${params.id}`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["fleet_status"] });

  // --- fleet_vote ---
  api.registerTool(() => ({
    name: "fleet_vote",
    label: "Fleet Vote",
    description: "Cast a vote on a fleet-wide SafeSwitch proposal. Approve or deny with optional reason.",
    parameters: {
      type: "object",
      properties: {
        id: { type: "string", description: "Fleet switch ID" },
        peer_id: { type: "string", description: "Your mesh peer ID" },
        approve: { type: "boolean", description: "true to approve, false to deny" },
        reason: { type: "string", description: "Optional reason for the vote" },
      },
      required: ["id", "peer_id", "approve"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await watchRequest("POST", `/fleet/vote/${params.id}`, {
          peer_id: params.peer_id,
          approve: params.approve,
          reason: params.reason,
        }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["fleet_vote"] });

  // --- fleet_rollback ---
  api.registerTool(() => ({
    name: "fleet_rollback",
    label: "Fleet Rollback",
    description: "Force rollback a fleet-wide SafeSwitch. Triggers rollback on all participating nodes.",
    parameters: {
      type: "object",
      properties: {
        id: { type: "string", description: "Fleet switch ID" },
      },
      required: ["id"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await watchRequest("POST", `/fleet/rollback/${params.id}`) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["fleet_rollback"] });

  // =========================================================================
  // Wallet Transaction Builder (via osmoda-keyd — real EIP-1559 + Solana tx)
  // =========================================================================

  // --- wallet_build_tx ---
  api.registerTool(() => ({
    name: "wallet_build_tx",
    label: "Build Transaction",
    description: "Build and sign a real blockchain transaction (EIP-1559 for Ethereum, legacy transfer for Solana). Returns the signed transaction bytes ready for broadcast. Does NOT broadcast — you decide when to send.",
    parameters: {
      type: "object",
      properties: {
        wallet_id: { type: "string", description: "Wallet ID to sign with" },
        chain: { type: "string", description: "'ethereum' or 'solana'" },
        to: { type: "string", description: "Recipient address" },
        amount: { type: "string", description: "Amount to send (wei for ETH, lamports for SOL)" },
        chain_params: {
          type: "object",
          description: "Chain-specific parameters",
          properties: {
            chain_id: { type: "number", description: "ETH chain ID (default: 1 mainnet)" },
            nonce: { type: "number", description: "ETH nonce (required for ETH)" },
            max_fee_per_gas: { type: "string", description: "ETH max fee per gas in wei" },
            max_priority_fee: { type: "string", description: "ETH max priority fee in wei" },
            gas_limit: { type: "number", description: "ETH gas limit (default: 21000)" },
            recent_blockhash: { type: "string", description: "SOL recent blockhash (required for SOL)" },
          },
        },
      },
      required: ["wallet_id", "chain", "to", "amount"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        return { output: await keydRequest("POST", "/wallet/build_tx", {
          wallet_id: params.wallet_id,
          chain: params.chain,
          tx_type: "transfer",
          to: params.to,
          amount: params.amount,
          chain_params: params.chain_params || {},
        }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["wallet_build_tx"] });

  // =========================================================================
  // App management tools (systemd transient services + JSON registry)
  // =========================================================================

  // --- app_deploy ---
  api.registerTool(() => ({
    name: "app_deploy",
    label: "Deploy App",
    description: "Deploy an application as a managed systemd service. Uses systemd-run with DynamicUser isolation by default. Resource limits (memory, CPU) and restart policy are configurable. The app is registered for boot persistence.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "App name (1-64 chars, alphanumeric + dash/underscore)" },
        command: { type: "string", description: "Absolute path to the binary or script to run" },
        args: { type: "array", items: { type: "string" }, description: "Command arguments" },
        working_dir: { type: "string", description: "Working directory (must exist)" },
        env: { type: "object", additionalProperties: { type: "string" }, description: "Environment variables" },
        memory_max: { type: "string", description: "Memory limit (e.g. '256M', '1G')" },
        cpu_quota: { type: "string", description: "CPU quota percentage (e.g. '50%', '200%' for 2 cores)" },
        restart_policy: { type: "string", description: "Restart policy: no, on-failure (default), on-abnormal, on-watchdog, on-abort, always" },
        port: { type: "number", description: "Primary port the app listens on (informational, for discovery)" },
        user: { type: "string", description: "Run as this user instead of DynamicUser (for filesystem access)" },
      },
      required: ["name", "command"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const name = String(params.name || "").trim();
        if (!name || name.length > 64 || !/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/.test(name)) {
          return { output: JSON.stringify({ error: "name must be 1-64 chars, alphanumeric start, only [a-zA-Z0-9_-]" }) };
        }
        const command = String(params.command || "").trim();
        if (!command || !command.startsWith("/")) {
          return { output: JSON.stringify({ error: "command must be an absolute path" }) };
        }

        const unitName = getAppUnitName(name);

        // Check if already running
        try {
          const state = child_process.execFileSync("systemctl", ["is-active", unitName], { timeout: 5000, encoding: "utf-8" }).trim();
          if (state === "active" || state === "activating") {
            return { output: JSON.stringify({ error: `app '${name}' is already running (unit: ${unitName})` }) };
          }
        } catch { /* not active, good */ }

        const workingDir = params.working_dir ? String(params.working_dir) : undefined;
        if (workingDir) {
          try { fs.accessSync(workingDir, fs.constants.R_OK); } catch {
            return { output: JSON.stringify({ error: `working_dir does not exist or is not readable: ${workingDir}` }) };
          }
        }

        const restartPolicy = validateRestartPolicy(params.restart_policy as string | undefined);
        const args: string[] = [
          "--unit", unitName,
          "--service-type=simple",
          `--property=Restart=${restartPolicy}`,
          "--property=StartLimitIntervalSec=0",
          "--property=RestartSec=3",
        ];

        if (params.user) {
          const user = String(params.user).replace(/[^a-zA-Z0-9_-]/g, "");
          if (user) args.push(`--uid=${user}`);
        } else {
          args.push("--property=DynamicUser=yes");
        }

        if (workingDir) args.push(`--working-directory=${workingDir}`);
        if (params.memory_max) args.push(`--property=MemoryMax=${String(params.memory_max)}`);
        if (params.cpu_quota) args.push(`--property=CPUQuota=${String(params.cpu_quota)}`);

        // Environment variables
        const env = (params.env && typeof params.env === "object") ? params.env as Record<string, string> : {};
        for (const [k, v] of Object.entries(env)) {
          const safeKey = k.replace(/[^a-zA-Z0-9_]/g, "");
          if (safeKey) args.push(`--setenv=${safeKey}=${v}`);
        }

        // The command + its args
        args.push("--", command);
        const cmdArgs = Array.isArray(params.args) ? (params.args as string[]).map(String) : [];
        args.push(...cmdArgs);

        const result = runExec("systemd-run", args, 15000);

        // Update registry
        const registry = loadAppRegistry();
        registry.apps[name] = {
          name,
          command,
          args: cmdArgs.length > 0 ? cmdArgs : undefined,
          working_dir: workingDir,
          env: Object.keys(env).length > 0 ? env : undefined,
          memory_max: params.memory_max ? String(params.memory_max) : undefined,
          cpu_quota: params.cpu_quota ? String(params.cpu_quota) : undefined,
          restart_policy: restartPolicy,
          port: typeof params.port === "number" ? params.port : undefined,
          user: params.user ? String(params.user) : undefined,
          created_at: new Date().toISOString(),
          status: "running",
        };
        saveAppRegistry(registry);

        // Log to agentd ledger
        try {
          await agentdRequest("POST", "/memory/ingest", {
            event: {
              category: "app_management", subcategory: "deploy", actor: "osmoda-bridge",
              summary: `Deployed app '${name}': ${command}`,
              detail: JSON.stringify({ name, command, unit: unitName, restart_policy: restartPolicy }),
              metadata: { tags: ["app", "deploy", name] },
            },
          });
        } catch { /* best-effort */ }

        const status = getAppStatus(unitName);
        return {
          output: JSON.stringify({
            deployed: true, name, unit: unitName, command,
            restart_policy: restartPolicy, ...status,
            systemd_run_output: result.trim(),
          }),
        };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["app_deploy"] });

  // --- app_list ---
  api.registerTool(() => ({
    name: "app_list",
    label: "List Apps",
    description: "List all managed applications with their status, resource usage, and configuration.",
    parameters: { type: "object", properties: {}, required: [] },
    async execute(_id: string, _params: Record<string, unknown>) {
      try {
        const registry = loadAppRegistry();
        const apps = Object.values(registry.apps).map((app) => {
          const unitName = getAppUnitName(app.name);
          const status = getAppStatus(unitName);
          return {
            name: app.name, unit: unitName, command: app.command,
            args: app.args, port: app.port, user: app.user,
            registry_status: app.status, created_at: app.created_at,
            limits: {
              memory_max: app.memory_max || null,
              cpu_quota: app.cpu_quota || null,
              restart_policy: app.restart_policy,
            },
            ...status,
          };
        });
        return { output: JSON.stringify({ apps, total: apps.length }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["app_list"] });

  // --- app_logs ---
  api.registerTool(() => ({
    name: "app_logs",
    label: "App Logs",
    description: "Retrieve journal logs for a managed application.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "App name" },
        lines: { type: "number", description: "Number of log lines (default 50, max 500)" },
        since: { type: "string", description: "Show logs since this time (e.g. '1 hour ago', '2024-01-01')" },
        priority: { type: "string", description: "Minimum priority: emerg, alert, crit, err, warning, notice, info, debug" },
      },
      required: ["name"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const name = String(params.name || "").trim();
        const registry = loadAppRegistry();
        if (!registry.apps[name]) {
          return { output: JSON.stringify({ error: `app '${name}' not found in registry` }) };
        }
        const unitName = getAppUnitName(name);
        const lines = Math.min(Math.max(parseInt(String(params.lines || "50"), 10) || 50, 1), 500);
        const args = ["-u", unitName, "-n", String(lines), "--no-pager", "-o", "short-iso"];
        if (params.since) args.push("--since", String(params.since));
        if (params.priority) args.push("-p", sanitizePriority(String(params.priority)));
        const logs = runExec("journalctl", args, 10000);
        return { output: JSON.stringify({ app: name, unit: unitName, lines: lines, logs: logs.trim() }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["app_logs"] });

  // --- app_stop ---
  api.registerTool(() => ({
    name: "app_stop",
    label: "Stop App",
    description: "Stop a running managed application. The app remains in the registry and can be restarted.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "App name" },
      },
      required: ["name"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const name = String(params.name || "").trim();
        const registry = loadAppRegistry();
        if (!registry.apps[name]) {
          return { output: JSON.stringify({ error: `app '${name}' not found in registry` }) };
        }
        const unitName = getAppUnitName(name);
        const result = runExec("systemctl", ["stop", unitName], 15000);

        registry.apps[name].status = "stopped";
        saveAppRegistry(registry);

        // Log to agentd ledger
        try {
          await agentdRequest("POST", "/memory/ingest", {
            event: {
              category: "app_management", subcategory: "stop", actor: "osmoda-bridge",
              summary: `Stopped app '${name}'`, detail: JSON.stringify({ name, unit: unitName }),
              metadata: { tags: ["app", "stop", name] },
            },
          });
        } catch { /* best-effort */ }

        return { output: JSON.stringify({ stopped: true, name, unit: unitName, output: result.trim() }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["app_stop"] });

  // --- app_restart ---
  api.registerTool(() => ({
    name: "app_restart",
    label: "Restart App",
    description: "Restart a managed application. If the unit is inactive, re-deploys from the registry.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "App name" },
      },
      required: ["name"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const name = String(params.name || "").trim();
        const registry = loadAppRegistry();
        const app = registry.apps[name];
        if (!app) {
          return { output: JSON.stringify({ error: `app '${name}' not found in registry` }) };
        }
        const unitName = getAppUnitName(name);

        // Check if unit is active — if so, use systemctl restart
        let isActive = false;
        try {
          const state = child_process.execFileSync("systemctl", ["is-active", unitName], { timeout: 5000, encoding: "utf-8" }).trim();
          isActive = (state === "active" || state === "activating");
        } catch { /* not active */ }

        let result: string;
        if (isActive) {
          result = runExec("systemctl", ["restart", unitName], 15000);
        } else {
          // Re-deploy from registry
          const args: string[] = [
            "--unit", unitName,
            "--service-type=simple",
            `--property=Restart=${app.restart_policy}`,
            "--property=StartLimitIntervalSec=0",
            "--property=RestartSec=3",
          ];
          if (app.user) {
            args.push(`--uid=${app.user}`);
          } else {
            args.push("--property=DynamicUser=yes");
          }
          if (app.working_dir) args.push(`--working-directory=${app.working_dir}`);
          if (app.memory_max) args.push(`--property=MemoryMax=${app.memory_max}`);
          if (app.cpu_quota) args.push(`--property=CPUQuota=${app.cpu_quota}`);
          if (app.env) {
            for (const [k, v] of Object.entries(app.env)) {
              const safeKey = k.replace(/[^a-zA-Z0-9_]/g, "");
              if (safeKey) args.push(`--setenv=${safeKey}=${v}`);
            }
          }
          args.push("--", app.command);
          if (app.args) args.push(...app.args);
          result = runExec("systemd-run", args, 15000);
        }

        registry.apps[name].status = "running";
        saveAppRegistry(registry);

        // Log to agentd ledger
        try {
          await agentdRequest("POST", "/memory/ingest", {
            event: {
              category: "app_management", subcategory: "restart", actor: "osmoda-bridge",
              summary: `Restarted app '${name}'`, detail: JSON.stringify({ name, unit: unitName, was_active: isActive }),
              metadata: { tags: ["app", "restart", name] },
            },
          });
        } catch { /* best-effort */ }

        const status = getAppStatus(unitName);
        return { output: JSON.stringify({ restarted: true, name, unit: unitName, re_deployed: !isActive, ...status, output: result.trim() }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["app_restart"] });

  // --- app_remove ---
  api.registerTool(() => ({
    name: "app_remove",
    label: "Remove App",
    description: "Stop and permanently remove a managed application from the registry.",
    parameters: {
      type: "object",
      properties: {
        name: { type: "string", description: "App name" },
      },
      required: ["name"],
    },
    async execute(_id: string, params: Record<string, unknown>) {
      try {
        const name = String(params.name || "").trim();
        const registry = loadAppRegistry();
        if (!registry.apps[name]) {
          return { output: JSON.stringify({ error: `app '${name}' not found in registry` }) };
        }
        const unitName = getAppUnitName(name);

        // Stop the unit (ignore errors if already stopped)
        runExec("systemctl", ["stop", unitName], 15000);

        // Remove from registry
        delete registry.apps[name];
        saveAppRegistry(registry);

        // Log to agentd ledger
        try {
          await agentdRequest("POST", "/memory/ingest", {
            event: {
              category: "app_management", subcategory: "remove", actor: "osmoda-bridge",
              summary: `Removed app '${name}'`, detail: JSON.stringify({ name, unit: unitName }),
              metadata: { tags: ["app", "remove", name] },
            },
          });
        } catch { /* best-effort */ }

        return { output: JSON.stringify({ removed: true, name, unit: unitName }) };
      } catch (e: any) {
        return { output: JSON.stringify({ error: e.message }) };
      }
    },
  }), { names: ["app_remove"] });
}
