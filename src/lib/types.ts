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

/** Whether a message carries work or is a courtesy. */
export type Intent = "work" | "courtesy";

export type Part =
  | { type: "text"; text: string }
  | { type: "json"; name: string; value: unknown }
  | { type: "notice"; kind: NoticeKind; text: string }
  | { type: "toolCall"; name: string; arguments: unknown; outcome: ToolOutcome }
  /**
   * A file the message carries. The bytes stay in the runtime's file store and
   * are addressed by `digest`; a transcript is read in bulk, so a document is
   * never inlined into one.
   */
  | { type: "file"; digest: string; name: string; mime: string; bytes: number }
  /**
   * An agent asking the operator for permission. Carries its own wording, so an
   * old channel still says what was asked; what came of it is read from
   * {@link Approval} state by `id`.
   */
  | {
      type: "approval";
      id: ApprovalId;
      action: ProtectedAction;
      summary: string;
      detail: DetailField[];
    };

export type ApprovalId = string;

/** Something an agent may not do without being told it can. */
export type ProtectedAction = "createAgent" | "actOnBehalf";

/** What the operator can answer. Pending and expired are not answers. */
export type Decision = "allow" | "alwaysAllow" | "deny";

export type ApprovalState = Decision | "pending" | "expired";

/**
 * One field of a request. `value` is what the model asked for, so it is
 * rendered as text and never as markdown.
 */
export interface DetailField {
  label: string;
  value: string;
}

export interface Approval {
  id: ApprovalId;
  agentId: AgentId;
  groupId: GroupId;
  runId: RunId;
  action: ProtectedAction;
  summary: string;
  detail: DetailField[];
  state: ApprovalState;
  createdAt: number;
  decidedAt: number | null;
}

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
  /**
   * What the sender said this message was for. Distinct from
   * {@link Envelope.expectsReply}: that says whether anybody is waiting on your
   * words, this says whether you were given something to do.
   */
  intent: Intent;
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

export type ConnectorId = string;

/**
 * A credential the whole group's machines are given.
 *
 * The value is never on this side of the boundary: there is no command that
 * returns one. `secretSet` and `secretHint` are all the UI ever sees.
 */
export interface Connector {
  id: ConnectorId;
  groupId: GroupId;
  /** What it is for: `GitHub`, `Linear`, `Stripe`. */
  service: string;
  /** Who it acts as, so the agent knows whose account it is using. */
  account: string;
  /** The environment variable the agent finds it in. */
  envVar: string;
  /** One line the agent reads: `read-only`, `production, do not write`. */
  note: string;
  secretSet: boolean;
  /** Last four characters, so two tokens can be told apart. Never the value. */
  secretHint: string;
  createdAt: number;
  updatedAt: number;
}

/**
 * There is no edit command: a credential is forgotten and re-added rather than
 * rewritten, so this is the only call that ever carries a value.
 */
export interface ConnectorDraft {
  groupId: GroupId;
  service: string;
  account: string;
  envVar: string;
  note: string;
  secret: string;
}

/**
 * A site an agent's browser turned out to be signed in to.
 *
 * Nobody types these. They are read off the machine by asking Chrome what
 * cookies it holds, so an agent signed in a minute ago advertises it without
 * anyone recording anything.
 */
export interface Signin {
  agentId: AgentId;
  /** The host, normalised: `linkedin.com`. */
  domain: string;
  /** A recognised service's real name, or the domain when it is a guess. */
  service: string;
  /** False when this came from the weaker visited-plus-session-cookie rule. */
  recognised: boolean;
  firstSeenAt: number;
  lastSeenAt: number;
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
  /** Kept at the top of the rail. Where the row is drawn, and nothing else. */
  pinned: boolean;
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
  /** Parked mid-turn on a permission request. Waiting on a person, not a model. */
  | { state: "awaitingApproval" }
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
  | { type: "runSettled"; runId: RunId; stepsUsed: number }
  | { type: "approvalRequested"; approvalId: ApprovalId; agentId: AgentId }
  | { type: "approvalSettled"; approvalId: ApprovalId; state: ApprovalState };

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
  /** Memories wiped. Named for the file on disk, which is `notes`. */
  notes: number;
  calls: number;
}

export type RoutineId = string;

/**
 * What makes a routine fire.
 *
 * `once`, `daily`, `weekdays`, `weekly`, `monthly`, or `every:<seconds>` for a
 * fixed gap. A string rather than a union of literals because `every:N` is
 * open-ended and because the trigger after these is a connector event, which
 * has to be a new value here rather than a new field. Read it with
 * `parseTrigger` in `lib/trigger.ts`; nothing branches on the raw text.
 */
export type TriggerSpec = string;

/** An agent's own schedule. Set by the agent, or by hand. */
export interface Routine {
  id: RoutineId;
  agentId: AgentId;
  /** What the operator calls it. Empty on anything an agent set unnamed. */
  name: string;
  /** The instruction, delivered to the agent when it fires. */
  what: string;
  trigger: TriggerSpec;
  /** Set up but not running. Everything else about it survives being off. */
  active: boolean;
  nextRunAt: number;
  lastRunAt: number | null;
  createdAt: number;
}

/** Why a routine ran. A test is the operator's button, not the clock. */
export type RunKind = "scheduled" | "test";

/**
 * One firing. `runId` threads back to everything it produced: the messages in
 * the channel and the model calls on the bill are both filed under it.
 */
export interface RoutineRun {
  runId: RunId;
  kind: RunKind;
  at: number;
}

/** Absent `inSecs` on an edit leaves the next firing where it was. */
export interface RoutineDraft {
  name: string;
  what: string;
  trigger: TriggerSpec;
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
