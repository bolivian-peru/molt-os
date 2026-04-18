/**
 * Agents config (/var/lib/osmoda/config/agents.json).
 *
 * In-memory cache + atomic writes + SIGHUP-driven reload. The gateway never
 * reads the file in hot paths; it reads the in-memory snapshot. reload()
 * swaps the snapshot atomically — in-flight sessions keep their closure
 * over the old snapshot; new sessions see the new one.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { CONFIG_DIR } from "./credentials.js";
import type { AgentProfile } from "./drivers/types.js";

export const AGENTS_FILE = path.join(CONFIG_DIR, "agents.json");

export interface ChannelBinding {
  channel: string;
  agent_id: string;
}

export interface AgentsFile {
  version: 1;
  agents: AgentProfile[];
  bindings: ChannelBinding[];
}

function emptyFile(): AgentsFile {
  return { version: 1, agents: [], bindings: [] };
}

export function loadAgentsFile(): AgentsFile {
  try {
    const raw = fs.readFileSync(AGENTS_FILE, "utf8");
    const parsed = JSON.parse(raw);
    if (parsed && parsed.version === 1 && Array.isArray(parsed.agents)) {
      parsed.bindings = Array.isArray(parsed.bindings) ? parsed.bindings : [];
      return parsed;
    }
  } catch { /* fall through */ }
  return emptyFile();
}

function atomicWrite(file: string, content: string, mode = 0o600): void {
  fs.mkdirSync(path.dirname(file), { recursive: true, mode: 0o700 });
  const tmp = `${file}.tmp-${process.pid}-${Date.now()}`;
  fs.writeFileSync(tmp, content, { mode });
  fs.renameSync(tmp, file);
}

export function saveAgentsFile(file: AgentsFile): void {
  if (file.version !== 1) file.version = 1;
  atomicWrite(AGENTS_FILE, JSON.stringify(file, null, 2), 0o640);
}

export class ConfigCache {
  private snapshot: AgentsFile;

  constructor() {
    this.snapshot = loadAgentsFile();
  }

  current(): AgentsFile { return this.snapshot; }

  reload(): AgentsFile {
    this.snapshot = loadAgentsFile();
    return this.snapshot;
  }

  findAgent(id: string): AgentProfile | undefined {
    return this.snapshot.agents.find((a) => a.id === id && a.enabled !== false);
  }

  agentForChannel(channel: string): AgentProfile | undefined {
    const binding = this.snapshot.bindings.find((b) => b.channel === channel);
    if (binding) {
      const a = this.findAgent(binding.agent_id);
      if (a) return a;
    }
    // Fall back to any web-capable agent.
    if (channel === "web") {
      return this.snapshot.agents.find((a) => a.enabled !== false && a.channels.includes("web"))
        || this.snapshot.agents.find((a) => a.enabled !== false);
    }
    return this.snapshot.agents.find((a) => a.enabled !== false);
  }

  upsertAgent(agent: AgentProfile): void {
    const idx = this.snapshot.agents.findIndex((a) => a.id === agent.id);
    if (idx >= 0) this.snapshot.agents[idx] = agent;
    else this.snapshot.agents.push(agent);
    saveAgentsFile(this.snapshot);
  }

  removeAgent(id: string): boolean {
    const idx = this.snapshot.agents.findIndex((a) => a.id === id);
    if (idx < 0) return false;
    this.snapshot.agents.splice(idx, 1);
    this.snapshot.bindings = this.snapshot.bindings.filter((b) => b.agent_id !== id);
    saveAgentsFile(this.snapshot);
    return true;
  }

  setBindings(bindings: ChannelBinding[]): void {
    this.snapshot.bindings = bindings;
    saveAgentsFile(this.snapshot);
  }
}
