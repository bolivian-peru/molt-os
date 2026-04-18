/**
 * Zero-downtime migration — runs once when agents.json is missing.
 *
 * Pre-v1.2 layouts:
 *   1. /var/lib/osmoda/config/api-key    → single Claude Code credential
 *   2. /root/.openclaw/agents/<id>/agent/auth-profiles.json → per-agent OpenClaw auth
 *
 * Post-migration: one credential per distinct secret, one AgentProfile per
 * legacy agent pointing to the matching credential. Existing daemons keep
 * running; this module only writes config files.
 */

import * as fs from "node:fs";
import * as path from "node:path";
import { addCredential, loadCredentials } from "./credentials.js";
import { loadAgentsFile, saveAgentsFile, AgentsFile } from "./config.js";
import type { AgentProfile, AuthType } from "./drivers/types.js";

const LEGACY_API_KEY = "/var/lib/osmoda/config/api-key";
const LEGACY_OPENCLAW_AGENTS = "/root/.openclaw/agents";
const BOOTSTRAP_CREDS = "/var/lib/osmoda/config/bootstrap-credentials.json";
const BOOTSTRAP_AGENTS = "/var/lib/osmoda/config/agents.json"; // written by install.sh

function detectType(secret: string): AuthType {
  return secret.startsWith("sk-ant-oat") ? "oauth" : "api_key";
}
function detectProvider(secret: string): "anthropic" | "openai" {
  return secret.startsWith("sk-ant-") ? "anthropic" : "openai";
}

export interface MigrationReport {
  ran: boolean;
  imported_credentials: number;
  created_agents: number;
  detected_runtime: "claude-code" | "openclaw" | "mixed" | "none";
  notes: string[];
}

export function runMigrationIfNeeded(): MigrationReport {
  const report: MigrationReport = {
    ran: false,
    imported_credentials: 0,
    created_agents: 0,
    detected_runtime: "none",
    notes: [],
  };

  // Bootstrap path (install.sh wrote a plaintext credentials file).
  // Always honor it on every boot — it deletes itself after absorbing.
  if (fs.existsSync(BOOTSTRAP_CREDS)) {
    try {
      const raw = JSON.parse(fs.readFileSync(BOOTSTRAP_CREDS, "utf8"));
      if (Array.isArray(raw.credentials)) {
        const existingCreds = loadCredentials().credentials;
        let imported = 0;
        for (const c of raw.credentials) {
          if (!c.secret) continue;
          // De-dupe by (label, provider, type, secret-prefix) to stay idempotent on re-runs.
          const prefix = c.secret.slice(0, 16);
          const dupe = existingCreds.find(
            (x) => x.label === c.label && x.provider === c.provider && x.type === c.type &&
                   x.secret.slice(0, 16) === prefix,
          );
          if (dupe) continue;
          addCredential({
            label: c.label || `${c.provider} ${c.type}`,
            provider: c.provider,
            type: c.type,
            secret: c.secret,
            base_url: c.base_url,
          });
          imported += 1;
        }
        report.imported_credentials += imported;
        report.notes.push(`absorbed ${imported} credential(s) from bootstrap file`);
      }
      fs.unlinkSync(BOOTSTRAP_CREDS);
    } catch (e) {
      report.notes.push(`bootstrap-credentials parse failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  }

  const existing = loadAgentsFile();
  if (existing.agents.length > 0) {
    // Even if agents.json already exists (typical after install.sh wrote it),
    // we still ran the credential absorption above. Report and return.
    if (report.imported_credentials > 0) {
      report.ran = true;
    } else {
      report.notes.push("agents.json already populated; migration skipped");
    }
    return report;
  }

  report.ran = true;
  const newAgents: AgentProfile[] = [];

  // 1. Claude Code legacy
  let claudeCredId: string | null = null;
  try {
    const secret = fs.readFileSync(LEGACY_API_KEY, "utf8").trim();
    if (secret) {
      const cred = addCredential({
        label: "Migrated Claude Code key",
        provider: detectProvider(secret),
        type: detectType(secret),
        secret,
      });
      claudeCredId = cred.id;
      report.imported_credentials += 1;
      report.notes.push(`imported api-key as credential ${cred.id}`);
    }
  } catch { /* not present — that's fine */ }

  // 2. OpenClaw legacy — one credential per distinct agent auth file.
  const openClawCredByAgent: Record<string, string> = {};
  try {
    if (fs.existsSync(LEGACY_OPENCLAW_AGENTS)) {
      for (const agentId of fs.readdirSync(LEGACY_OPENCLAW_AGENTS)) {
        const authFile = path.join(LEGACY_OPENCLAW_AGENTS, agentId, "agent", "auth-profiles.json");
        if (!fs.existsSync(authFile)) continue;
        try {
          const profile = JSON.parse(fs.readFileSync(authFile, "utf8"));
          const secret = profile.token || profile.key || "";
          if (!secret || secret.length < 10) {
            report.notes.push(`skipped openclaw agent ${agentId}: empty/invalid secret`);
            continue;
          }
          const cred = addCredential({
            label: `Migrated OpenClaw (${agentId})`,
            provider: profile.provider || detectProvider(secret),
            type: profile.type === "token" ? "oauth" : "api_key",
            secret,
          });
          openClawCredByAgent[agentId] = cred.id;
          report.imported_credentials += 1;
          report.notes.push(`imported openclaw agent ${agentId} as credential ${cred.id}`);
        } catch (e) {
          report.notes.push(`failed openclaw agent ${agentId}: ${e instanceof Error ? e.message : String(e)}`);
        }
      }
    }
  } catch { /* ignore */ }

  // Determine runtime used today.
  const gatewayUnit = safeRead("/etc/systemd/system/osmoda-gateway.service") ||
                      safeRead("/run/systemd/system/osmoda-gateway.service");
  const runtimeDetected: "claude-code" | "openclaw" =
    gatewayUnit && /openclaw\b/i.test(gatewayUnit) ? "openclaw" : "claude-code";
  report.detected_runtime = runtimeDetected;

  // Pick a credential to default agents to.
  const creds = loadCredentials().credentials;
  const defaultCredId =
    openClawCredByAgent["osmoda"] ||
    claudeCredId ||
    creds[0]?.id ||
    null;

  // Build default agent profiles.
  const now = new Date().toISOString();
  const mkAgent = (
    id: string,
    displayName: string,
    model: string,
    channels: string[],
    runtime: "claude-code" | "openclaw",
    credentialId: string | null,
  ): AgentProfile => ({
    id,
    display_name: displayName,
    runtime,
    credential_id: credentialId || "",
    model,
    channels,
    profile_dir: `/var/lib/osmoda/workspace-${id}`,
    enabled: Boolean(credentialId),
    updated_at: now,
  });

  // osmoda: full-access web agent on Opus.
  newAgents.push(
    mkAgent(
      "osmoda",
      "osModa (full access)",
      "claude-opus-4-6",
      ["web", "api"],
      runtimeDetected,
      openClawCredByAgent["osmoda"] || defaultCredId,
    ),
  );
  // mobile: concise, Sonnet, Telegram/WhatsApp.
  newAgents.push(
    mkAgent(
      "mobile",
      "osModa mobile",
      "claude-sonnet-4-6",
      ["telegram", "whatsapp"],
      runtimeDetected,
      openClawCredByAgent["mobile"] || defaultCredId,
    ),
  );

  const file: AgentsFile = {
    version: 1,
    agents: newAgents,
    bindings: [
      { channel: "telegram", agent_id: "mobile" },
      { channel: "whatsapp", agent_id: "mobile" },
    ],
  };
  saveAgentsFile(file);
  report.created_agents = newAgents.length;
  return report;
}

function safeRead(p: string): string | null {
  try { return fs.readFileSync(p, "utf8"); } catch { return null; }
}
