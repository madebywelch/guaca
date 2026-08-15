/**
 * Wire types. These mirror the Rust structs in `src-tauri/src/domain` and
 * `src-tauri/src/runtime/events.rs` exactly. Everything crossing the IPC
 * boundary is camelCase; if a field here is snake_case, one side is wrong.
 */

export type AgentId = string;
export type GroupId = string;
export type MessageId = string;
export type RunId = string;

export type Lifecycle = "active" | "paused" | "terminated";
export type Trust = "operator" | "peer" | "system";
export type NoticeKind = "guardStop" | "upstreamError" | "lifecycle";

export type Participant = { kind: "human" } | { kind: "agent"; id: AgentId } | { kind: "system" };

export interface RefusedRecipient {
  to: string;
  reason: string;
}

export type ToolOutcome =
  | { status: "ok"; summary: string }
  /** A fan-out where some recipients took it and some did not. */
  | { status: "partial"; summary: string; refused: RefusedRecipient[] }
  | { status: "refused"; reason: string }
  | { status: "failed"; error: string };

export type Part =
  | { type: "text"; text: string }
  | { type: "json"; name: string; value: unknown }
  | { type: "notice"; kind: NoticeKind; text: string }
  | { type: "toolCall"; name: string; arguments: unknown; outcome: ToolOutcome };

export interface Envelope {
  id: MessageId;
  runId: RunId;
  channelId: AgentId;
  from: Participant;
  to: Participant;
  parts: Part[];
  trust: Trust;
  hop: number;
  expectsReply: boolean;
  cause: MessageId | null;
  createdAt: number;
}

/**
 * An isolation boundary. Agents in different groups cannot see or message each
 * other; the wall is enforced in the Rust runtime, not here. The UI keeps
 * groups out of the way entirely while only the default one exists.
 */
export interface Group {
  id: GroupId;
  name: string;
  /** Live agents in it. Terminated ones are excluded. */
  agentCount: number;
  createdAt: number;
  /** `null` means inherit the app default. Settings resolve agent → group → app. */
  baseUrl: string | null;
  defaultModel: string | null;
  apiKeySet: boolean;
  apiKeyHint: string;
}

/** Absent fields are left as they were; empty strings clear the override. */
export interface GroupDraft {
  name: string;
  baseUrl?: string;
  defaultModel?: string;
  apiKey?: string;
}

/** An agent's sandbox: a Linux machine with a shell, a network and a desktop. */
export interface Computer {
  sandboxId: string;
  /** `running`, `asleep` (disk kept, wakes on use) or `gone`. */
  state: string;
  /** Absent until the desktop processes are up inside the sandbox. */
  vncUrl: string | null;
}

/** One command's result, from the agent's computer. */
export interface Output {
  stdout: string;
  stderr: string;
  exitCode: number;
}

export interface AgentCard {
  id: AgentId;
  groupId: GroupId;
  /** Set once the agent has been given a computer. */
  sandboxId: string | null;
  name: string;
  avatar: string;
  color: string;
  model: string;
  systemPrompt: string;
  skills: string[];
  lifecycle: Lifecycle;
  version: number;
  createdAt: number;
  updatedAt: number;
}

export interface AgentDraft {
  /** Omitted means "leave it where it is" on update, "default group" on create. */
  groupId?: GroupId;
  name: string;
  avatar: string;
  color: string;
  model: string;
  systemPrompt: string;
  skills: string[];
}

export type Activity =
  | { state: "idle" }
  | { state: "thinking" }
  | { state: "queued"; depth: number }
  | { state: "paused" };

export interface GuardLimits {
  maxHops: number;
  maxStepsPerRun: number;
  maxFanoutPerCall: number;
  maxSendsPerPair: number;
  /** Model calls inside one turn as an agent works through tool results. */
  maxToolRounds: number;
}

export interface Settings {
  /** What agents call you. Empty means they say "the operator". */
  operatorName: string;
  e2bKeySet: boolean;
  e2bKeyHint: string;
  computerIdleMinutes: number;
  baseUrl: string;
  defaultModel: string;
  apiKeySet: boolean;
  apiKeyHint: string;
  requestTimeoutSecs: number;
  limits: GuardLimits;
}

/** Absent fields are left unchanged. An empty `apiKey` clears the key. */
export interface SettingsPatch {
  operatorName?: string;
  e2bApiKey?: string;
  computerIdleMinutes?: number;
  baseUrl?: string;
  apiKey?: string;
  defaultModel?: string;
  requestTimeoutSecs?: number;
  limits?: GuardLimits;
}

export type UiEvent =
  | { type: "agentsChanged" }
  | { type: "messageAppended"; message: Envelope }
  | {
      type: "streamStarted";
      messageId: MessageId;
      channelId: AgentId;
      agentId: AgentId;
      runId: RunId;
      /** Decides whether the UI draws a bubble or a quiet "writing" line. */
      to: Participant;
    }
  | { type: "streamDelta"; messageId: MessageId; channelId: AgentId; text: string }
  | { type: "streamEnded"; messageId: MessageId; channelId: AgentId }
  | { type: "activityChanged"; agentId: AgentId; activity: Activity }
  | { type: "channelsCleared"; agents: AgentId[] }
  | {
      type: "tokensUsed";
      agentId: AgentId;
      groupId: GroupId;
      runId: RunId;
      prompt: number;
      completion: number;
      /** Null when the provider does not price calls. Not the same as free. */
      cost: number | null;
    }
  | { type: "runSettled"; runId: RunId; stepsUsed: number };

/** Tokens spent, as the provider counted them. Never estimated. */
export interface Tokens {
  prompt: number;
  completion: number;
  /** Dollars, when the provider prices calls. Null for a local server. */
  cost: number | null;
  /** Model calls, not agent turns: one turn can make several. */
  calls: number;
}

export interface GroupUsage extends Tokens {
  groupId: GroupId;
}

export interface RunUsage extends Tokens {
  runId: RunId;
}

/** What a reset took. Reported rather than assumed. */
export interface GroupReset {
  messages: number;
  routines: number;
  notes: number;
  calls: number;
}

export type RoutineId = string;

/** An agent's own schedule. Set by the agent, or by hand. */
export interface Routine {
  id: RoutineId;
  agentId: AgentId;
  /** The instruction, delivered to the agent when it fires. */
  what: string;
  /** Null fires once and is done. */
  everySecs: number | null;
  nextRunAt: number;
  lastRunAt: number | null;
  createdAt: number;
}

/** Absent `inSecs` on an edit leaves the next firing where it was. */
export interface RoutineDraft {
  what: string;
  everySecs: number | null;
  inSecs: number | null;
}

/** Structured error from a command. `kind` is safe to branch on. */
export interface CommandError {
  kind:
    | "validation"
    | "duplicateName"
    | "notFound"
    | "terminated"
    | "storage"
    | "config"
    | "inference";
  message: string;
}

export function isCommandError(value: unknown): value is CommandError {
  return (
    typeof value === "object" &&
    value !== null &&
    "kind" in value &&
    "message" in value &&
    typeof (value as CommandError).message === "string"
  );
}

/** Human-readable message for anything a command can throw. */
export function errorMessage(value: unknown): string {
  if (isCommandError(value)) return value.message;
  if (value instanceof Error) return value.message;
  if (typeof value === "string") return value;
  return "Something went wrong.";
}

/** Concatenated text of an envelope, matching the Rust `plain_text`. */
export function plainText(envelope: Envelope): string {
  return envelope.parts
    .filter((p): p is Extract<Part, { type: "text" }> => p.type === "text")
    .map((p) => p.text)
    .join("\n")
    .trim();
}

export function isInterAgent(envelope: Envelope): boolean {
  return envelope.from.kind === "agent" && envelope.to.kind === "agent";
}
