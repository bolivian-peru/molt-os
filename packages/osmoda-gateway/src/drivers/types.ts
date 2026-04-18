/**
 * Driver interface — one file per runtime implementation.
 *
 * Adding a new runtime (Codex, Bedrock, Vertex, generic-OpenAI, …) means
 * dropping a single module in this directory that exports a RuntimeDriver.
 * No changes elsewhere in the gateway.
 */

export type Provider = "anthropic" | "openai" | "openrouter" | "deepseek" | string;
export type AuthType = "oauth" | "api_key";

export interface Credential {
  id: string;
  label: string;
  provider: Provider;
  type: AuthType;
  /** Decrypted secret. Only present in memory; credentials.json.enc stores the ciphertext. */
  secret: string;
  base_url?: string;
  created_at: string;
  last_tested_at?: string | null;
  last_test_ok?: boolean;
  last_test_error?: string | null;
  last_used_at?: string | null;
}

export interface AgentProfile {
  id: string;
  display_name: string;
  runtime: string;                   // matches RuntimeDriver.name
  credential_id: string;
  model: string;
  channels: string[];
  profile_dir?: string;              // for SOUL.md/AGENTS.md/USER.md
  system_prompt_file?: string;
  enabled: boolean;
  updated_at: string;
}

export interface AgentEvent {
  type: "text" | "tool_use" | "tool_result" | "thinking" | "session" | "error" | "done";
  text?: string;
  name?: string;
  sessionId?: string;
  code?: string;
}

export interface DriverSessionOpts {
  agent: AgentProfile;
  credential: Credential;
  model: string;
  systemPrompt: string;
  mcpConfigPath: string;
  message: string;
  sessionId?: string;
  abortSignal?: AbortSignal;
  workingDir?: string;
}

export interface CredentialTestResult {
  ok: boolean;
  error?: string;
  /** Some providers can report which models this credential unlocks. */
  model_list?: string[];
}

export interface RuntimeDriver {
  readonly name: string;
  readonly displayName: string;
  readonly description: string;
  readonly supportedProviders: Provider[];
  readonly supportedAuthTypes: AuthType[];
  readonly defaultModels: string[];

  /** Non-destructive — should succeed with a 1-token ping or equivalent cheap check. */
  testCredential(cred: Credential): Promise<CredentialTestResult>;

  startSession(opts: DriverSessionOpts): AsyncGenerator<AgentEvent>;
}
