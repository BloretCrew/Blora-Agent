export interface ProviderConfig {
  kind: "mock" | "scripted" | "openai-compatible" | "openai" | "anthropic" | "responses" | "gemini" | "env";
  baseUrl?: string;
  apiKey?: string;
  model?: string;
  completions?: unknown[];
}

export interface ToolDefinition {
  name: string;
  description?: string;
  parameters?: Record<string, unknown>;
  readOnly?: boolean;
  requiresApproval?: boolean;
  handler?: (args: Record<string, unknown>) => unknown | Promise<unknown>;
}

export interface AgentOptions {
  binaryPath?: string;
  profile?: "full" | "web" | "read-only";
  persistence?: { kind: "memory" } | { kind: "file"; path: string };
  provider?: ProviderConfig;
  workspace?: { kind: "ephemeral" } | string;
  callbacks?: {
    approval?: "manual" | "deny" | "allow" | "read-only";
    question?: "manual" | "dismiss";
  };
  tools?: ToolDefinition[];
  onApproval?: (request: unknown) => unknown | Promise<unknown>;
  onQuestion?: (request: unknown) => string | null | Promise<string | null>;
  onError?: (error: unknown) => void;
}

export interface EventEnvelope {
  event_id: string;
  schema_version: number;
  session_id: string;
  run_id?: string | null;
  sequence: number;
  type: string;
  payload: unknown;
  [key: string]: unknown;
}

export interface RunOutcome {
  runId: string;
  status: string;
  finalText: string;
  usage: { inputTokens: number; outputTokens: number; cachedTokens: number };
  lastSequence: number;
}

export class Run {
  readonly id: string;
  readonly startSequence: number;
  sessionId?: string;
  events(): AsyncIterable<EventEnvelope>;
  wait(options?: { timeoutMs?: number }): Promise<RunOutcome>;
  cancel(): Promise<void>;
  steer(message: string): Promise<void>;
}

export class Session {
  readonly id: string;
  run(prompt: string, options?: {
    model?: string;
    maxTurns?: number;
    permission?: string;
    outputSchema?: unknown;
    attachments?: unknown[];
  }): Promise<Run>;
  events(afterSequence?: number): Promise<EventEnvelope[]>;
  project(): Promise<unknown>;
  resolveApproval(approvalId: string, allow: boolean): Promise<void>;
  answerInput(requestId: string, answer: string | null): Promise<void>;
  steer(message: string): Promise<void>;
}

export class Agent {
  static start(options?: AgentOptions): Promise<Agent>;
  createSession(spec?: {
    title?: string;
    mode?: string;
    workspace?: string;
    permission?: string;
  }): Promise<Session>;
  openSession(sessionId: string): Promise<Session>;
  listSessions(): Promise<unknown[]>;
  request(method: string, params?: Record<string, unknown>): Promise<unknown>;
  close(): Promise<void>;
}

export class BloraError extends Error {
  data?: unknown;
}
