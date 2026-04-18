/**
 * REST config endpoints — mounted by index.ts at /config/*.
 *
 * All endpoints require the gateway bearer token (same one used for WS auth).
 * Writes are atomic + trigger SIGHUP to self, so the gateway reloads config
 * without dropping WS clients.
 */

import type { IncomingMessage, ServerResponse } from "node:http";
import {
  loadCredentials, addCredential, removeCredential, setDefault,
  getCredential, updateCredentialMeta, redact,
} from "./credentials.js";
import { type ConfigCache, saveAgentsFile } from "./config.js";
import type { AgentProfile } from "./drivers/types.js";
import { getDriver, listDrivers } from "./drivers/index.js";

export interface ConfigApiDeps {
  cache: ConfigCache;
  authToken: string | null;
  reloadSelf: () => void;
}

function ok(res: ServerResponse, body: any, status = 200): void {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}
function err(res: ServerResponse, status: number, code: string, message: string): void {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify({ code, message, error: code }));
}

async function readJson(req: IncomingMessage): Promise<any> {
  let body = "";
  for await (const chunk of req) {
    body += chunk;
    if (body.length > 256 * 1024) throw new Error("payload too large");
  }
  if (!body) return {};
  return JSON.parse(body);
}

function authed(req: IncomingMessage, token: string | null): boolean {
  if (!token) return false;
  const h = req.headers.authorization;
  return typeof h === "string" && h === `Bearer ${token}`;
}

/**
 * Returns true if `url.pathname` started with /config/ and the request was
 * handled (response sent). Otherwise returns false so index.ts can fall
 * through to the rest of its routing.
 */
export async function handleConfigRequest(
  req: IncomingMessage,
  res: ServerResponse,
  url: URL,
  deps: ConfigApiDeps,
): Promise<boolean> {
  if (!url.pathname.startsWith("/config/") && url.pathname !== "/config") return false;
  if (!authed(req, deps.authToken)) {
    err(res, 401, "unauthorized", "Missing or invalid bearer token");
    return true;
  }

  try {
    // ── Drivers (read-only) ──────────────────────────────────────────────
    if (url.pathname === "/config/drivers" && req.method === "GET") {
      const drivers = listDrivers().map((d) => ({
        name: d.name,
        display_name: d.displayName,
        description: d.description,
        supported_providers: d.supportedProviders,
        supported_auth_types: d.supportedAuthTypes,
        default_models: d.defaultModels,
      }));
      ok(res, { drivers });
      return true;
    }

    // ── Credentials CRUD ────────────────────────────────────────────────
    if (url.pathname === "/config/credentials" && req.method === "GET") {
      const file = loadCredentials();
      ok(res, {
        default_credential_id: file.default_credential_id,
        credentials: file.credentials.map(redact),
      });
      return true;
    }
    if (url.pathname === "/config/credentials" && req.method === "POST") {
      const body = await readJson(req);
      if (!body.secret || typeof body.secret !== "string" || body.secret.length < 10) {
        return err(res, 400, "validation_failed", "secret required, min 10 chars"), true;
      }
      if (!body.provider) return err(res, 400, "validation_failed", "provider required"), true;
      if (!body.type) return err(res, 400, "validation_failed", "type required"), true;
      const cred = addCredential({
        label: body.label || `${body.provider} ${body.type}`,
        provider: body.provider,
        type: body.type,
        secret: body.secret,
        base_url: body.base_url,
      });
      deps.reloadSelf();
      ok(res, { credential: redact(cred) }, 201);
      return true;
    }
    const credTestMatch = url.pathname.match(/^\/config\/credentials\/([^/]+)\/test$/);
    if (credTestMatch && req.method === "POST") {
      const id = credTestMatch[1];
      const cred = getCredential(id);
      if (!cred) return err(res, 404, "not_found", "credential not found"), true;
      // Run test against every driver that claims support for this {provider,type}.
      const drivers = listDrivers().filter(
        (d) => d.supportedProviders.includes(cred.provider) &&
               d.supportedAuthTypes.includes(cred.type),
      );
      if (drivers.length === 0) {
        updateCredentialMeta(id, {
          last_tested_at: new Date().toISOString(),
          last_test_ok: false,
          last_test_error: "no driver accepts this provider+type",
        });
        return ok(res, { ok: false, error: "no driver accepts this provider+type" }), true;
      }
      const [first] = drivers;
      const result = await first.testCredential(cred);
      updateCredentialMeta(id, {
        last_tested_at: new Date().toISOString(),
        last_test_ok: result.ok,
        last_test_error: result.ok ? null : (result.error || null),
      });
      ok(res, result);
      return true;
    }
    const credDefaultMatch = url.pathname.match(/^\/config\/credentials\/([^/]+)\/default$/);
    if (credDefaultMatch && req.method === "POST") {
      const id = credDefaultMatch[1];
      if (!setDefault(id)) return err(res, 404, "not_found", "credential not found"), true;
      deps.reloadSelf();
      ok(res, { default_credential_id: id });
      return true;
    }
    const credIdMatch = url.pathname.match(/^\/config\/credentials\/([^/]+)$/);
    if (credIdMatch && req.method === "DELETE") {
      const id = credIdMatch[1];
      if (!removeCredential(id)) return err(res, 404, "not_found", "credential not found"), true;
      deps.reloadSelf();
      res.writeHead(204); res.end();
      return true;
    }
    if (credIdMatch && req.method === "PATCH") {
      const id = credIdMatch[1];
      const body = await readJson(req);
      const patch: any = {};
      if (typeof body.label === "string") patch.label = body.label;
      if (!updateCredentialMeta(id, patch)) return err(res, 404, "not_found", "credential not found"), true;
      ok(res, { credential: redact(getCredential(id)!) });
      return true;
    }

    // ── Agents CRUD ─────────────────────────────────────────────────────
    if (url.pathname === "/config/agents" && req.method === "GET") {
      ok(res, deps.cache.current());
      return true;
    }
    if (url.pathname === "/config/agents" && req.method === "PUT") {
      const body = await readJson(req);
      if (!Array.isArray(body.agents)) return err(res, 400, "validation_failed", "agents[] required"), true;
      // Validate each agent.
      const bad = (body.agents as AgentProfile[]).find((a) =>
        !a.id || !a.runtime || !a.model || typeof a.enabled !== "boolean");
      if (bad) return err(res, 400, "validation_failed", "agent missing required fields"), true;
      const creds = loadCredentials().credentials;
      for (const a of body.agents as AgentProfile[]) {
        if (a.credential_id && !creds.some((c) => c.id === a.credential_id)) {
          return err(res, 400, "validation_failed", `credential ${a.credential_id} not found`), true;
        }
        if (!getDriver(a.runtime)) {
          return err(res, 400, "validation_failed", `unknown runtime ${a.runtime}`), true;
        }
      }
      const now = new Date().toISOString();
      const normalized: AgentProfile[] = body.agents.map((a: AgentProfile) => ({
        ...a,
        updated_at: now,
        channels: Array.isArray(a.channels) ? a.channels : [],
      }));
      const bindings = Array.isArray(body.bindings) ? body.bindings : deps.cache.current().bindings;
      const current = deps.cache.current();
      current.agents = normalized;
      current.bindings = bindings;
      saveAgentsFile(current);
      deps.reloadSelf();
      ok(res, deps.cache.current());
      return true;
    }
    const agentIdMatch = url.pathname.match(/^\/config\/agents\/([^/]+)$/);
    if (agentIdMatch && req.method === "PATCH") {
      const id = agentIdMatch[1];
      const body = await readJson(req);
      const current = deps.cache.current();
      const agent = current.agents.find((a) => a.id === id);
      if (!agent) return err(res, 404, "not_found", "agent not found"), true;
      for (const k of ["runtime", "credential_id", "model", "display_name", "enabled", "channels", "profile_dir", "system_prompt_file"]) {
        if (k in body) (agent as any)[k] = body[k];
      }
      agent.updated_at = new Date().toISOString();
      if (!getDriver(agent.runtime)) return err(res, 400, "validation_failed", `unknown runtime ${agent.runtime}`), true;
      saveAgentsFile(current);
      deps.reloadSelf();
      ok(res, agent);
      return true;
    }
    if (agentIdMatch && req.method === "DELETE") {
      const id = agentIdMatch[1];
      if (!deps.cache.removeAgent(id)) return err(res, 404, "not_found", "agent not found"), true;
      deps.reloadSelf();
      res.writeHead(204); res.end();
      return true;
    }

    // ── Reload + health ─────────────────────────────────────────────────
    if (url.pathname === "/config/reload" && req.method === "POST") {
      deps.reloadSelf();
      ok(res, { ok: true });
      return true;
    }
    if (url.pathname === "/config/health" && req.method === "GET") {
      const creds = loadCredentials();
      ok(res, {
        agents_count: deps.cache.current().agents.length,
        enabled_agents_count: deps.cache.current().agents.filter((a) => a.enabled).length,
        credentials_count: creds.credentials.length,
        default_credential_id: creds.default_credential_id,
        drivers_count: listDrivers().length,
      });
      return true;
    }

    err(res, 404, "not_found", `no config route for ${req.method} ${url.pathname}`);
    return true;
  } catch (e: any) {
    err(res, 500, "internal_error", e?.message || String(e));
    return true;
  }
}

