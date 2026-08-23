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

/**
 * A file, as everything that refers to one refers to it.
 *
 * The bytes are not here and never cross IPC. They sit once in the runtime's
 * file store addressed by `digest`, which is the SHA-256 of the contents, and
 * the webview reads them over the `guacfile:` scheme when it has to draw one.
 * See `lib/files.ts`.
 */
export interface Attachment {
  digest: string;
  name: string;
  mime: string;
  bytes: number;
}

/** What became of a drop: what was taken, and what could not be. */
export interface Staged {
  attached: Attachment[];
  /** One line per refused file, saying which it was and why. */
  refused: string[];
}

export type Part =
  | { type: "text"; text: string }
  | { type: "json"; name: string; value: unknown }
  | { type: "notice"; kind: NoticeKind; text: string }
  /**
   * A call this agent made. `replaced` is what it overwrote, carried only by a
   * memory rewrite and absent everywhere else, including on calls recorded
   * before it existed. Empty means the call overwrote nothing, which is not the
   * same as a call that overwrites nothing.
   */
  | {
      type: "toolCall";
      name: string;
      arguments: unknown;
      outcome: ToolOutcome;
      replaced?: string;
    }
  | ({ type: "file" } & Attachment)
  /**
   * A routine coming due, drawn as one line the operator can open rather than
   * as dialogue. The instruction is in `what` and is what the model was sent;
   * `name` is the routine's name at the moment it fired, so a routine since
   * renamed does not rewrite what the transcript said it was.
   */
  | { type: "routine"; routineId: RoutineId; name: string; what: string }
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

/**
 * The one part with two lives: drawn as a chip while the turn is making the
 * call, and again out of the message that records it. Named because the live
 * half is carried whole by `toolFinished`, so both are built from one value.
 */
export type ToolCallPart = Extract<Part, { type: "toolCall" }>;

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
  /** How this group's turns are paid for and answered. Settings resolve agent →
   *  group → app, so `null` anywhere in here means the app decides. */
  inference: InferenceOverrides;
  apiKeySet: boolean;
  apiKeyHint: string;
  /** How far a conversation started in this group may run. */
  limits: GroupLimits;
}

/** Every field `null` is a group that runs on the app settings. */
export interface InferenceOverrides {
  provider: Provider | null;
  baseUrl: string | null;
  /** The model used when a key is paying. */
  defaultModel: string | null;
  /** The model used when the subscription is paying. Two fields for the reason
   *  the app keeps two: the providers have disjoint model names. */
  subscriptionModel: string | null;
  requestTimeoutSecs: number | null;
}

/** Per-field overrides of the app's loop guard. `null` inherits. */
export interface GroupLimits {
  maxHops: number | null;
  maxStepsPerRun: number | null;
  maxFanoutPerCall: number | null;
  maxSendsPerPair: number | null;
  maxToolRounds: number | null;
}

/**
 * What an operator can set on a group.
 *
 * Each block is all-or-nothing: absent leaves every override in it as it was,
 * and present replaces the lot, with a null field inside meaning inherit. The
 * key is the exception, because it is the one setting that cannot be read back:
 * absent keeps the stored one and `""` clears it.
 */
export interface GroupDraft {
  name: string;
  inference?: InferenceOverrides;
  apiKey?: string;
  limits?: GroupLimits;
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

export type PluginId = string;

/** The servers Guaca knows how to sign in to. Closed, and the same everywhere. */
export type PluginKind = "neon" | "cloudflare" | "linear" | "stripe" | "agentmail" | "google";

/** A plugin on offer, before anybody has connected it. */
export interface PluginOffer {
  kind: PluginKind;
  name: string;
  /** One line about what the crew gets. */
  blurb: string;
  docs: string;
  /** Where the sign-in and every later call goes, shown before it is clicked. */
  endpoint: string;
}

/**
 * Who in a crew may call one plugin's tools.
 *
 * The sign-in belongs to the group either way; this is who is allowed to spend
 * it. Two shapes rather than a list that means everybody when it is empty:
 * `everyone` covers agents that do not exist yet, and an empty `chosen` is a
 * plugin nobody may call, which is where an operator is standing the moment
 * before they tick the first name.
 */
export type PluginAccess = { mode: "everyone" } | { mode: "chosen"; agents: AgentId[] };

/**
 * One of a connected plugin's tools, and what the operator decided about it.
 *
 * The description and not the schema: an operator deciding whether the crew may
 * call `delete_customer` needs the sentence the vendor wrote about it and has
 * no use for the shape of its arguments. `allowed` is true unless it was
 * switched off, which is the way round the store keeps it too, so a tool a
 * vendor ships next month arrives switched on rather than invisible.
 */
export interface PluginToolCard {
  /** The server's own name for it. Prefixed with the plugin, a model calls it. */
  name: string;
  description: string;
  allowed: boolean;
}

/**
 * A plugin a group has connected.
 *
 * The grant is never on this side of the boundary: there is no command that
 * returns an access token, a refresh token or a client secret. `signedIn` is
 * all the UI ever sees, and it is false for a server that asked for nothing.
 */
export interface Plugin {
  id: PluginId;
  groupId: GroupId;
  kind: PluginKind;
  /** Whose account, when the server said. Usually blank. */
  account: string;
  /**
   * Every tool the server published, switched off ones included: a list that
   * left them out would be a panel with no way to switch one back on.
   */
  tools: PluginToolCard[];
  /** Which of the crew is offered them. `everyone` until somebody says else. */
  access: PluginAccess;
  signedIn: boolean;
  connectedAt: number;
}

/**
 * Which of an agent's two places holds a session.
 *
 * A computer and a browser have unrelated cookie jars, so a sign-in in one is
 * not reachable from the other. The operator needs this to know which window to
 * sign in through.
 */
export type Surface = "computer" | "browser";

/**
 * A site an agent turned out to be signed in to.
 *
 * Nobody types these. They are read off whatever holds the cookies, so an agent
 * signed in a minute ago advertises it without anyone recording anything.
 */
export interface Signin {
  agentId: AgentId;
  surface: Surface;
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

/**
 * An agent's hosted browser: a Chrome and nothing else.
 *
 * A different thing from a computer and on a different provider. There is no
 * asleep state to show: a browser goes to standby on its own within seconds and
 * comes back the moment anything drives it, so the operator has nothing to act
 * on.
 */
export interface Browser {
  sessionId: string;
  /** `running` or `gone`. */
  state: string;
  /** Where the operator watches and takes over. Absent once it has gone. */
  liveViewUrl: string | null;
  /**
   * Where a live view this window may not frame is served from.
   *
   * Set instead of `liveViewUrl`, never beside it. A frame the CSP refuses
   * draws the surface behind it and reports nothing, so the pane says which
   * address it was rather than showing a blank rectangle.
   */
  unwatchable: string | null;
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
  /** The machine this agent is using, once it has needed one. */
  sandboxId: string | null;
  /** The hosted browser it is using, which is a separate thing. */
  browserId: string | null;
  /**
   * Whether the operator has given this agent a computer at all.
   *
   * A different question from `sandboxId`, which is only what it is holding
   * right now: machines are reclaimed and remade, and this outlives all of
   * them. False means no tool that reaches a machine is offered to its turns,
   * and it cannot make one.
   */
  hasComputer: boolean;
  /** The same decision about the browser, and separately. */
  hasBrowser: boolean;
  name: string;
  avatar: string;
  color: string;
  model: string;
  systemPrompt: string;
  skills: string[];
  lifecycle: Lifecycle;
  /** Kept at the top of the rail. Where the row is drawn, and nothing else. */
  pinned: boolean;
  /**
   * Where the operator put this row. Lower is higher up its section.
   *
   * The arrangement, not the drawn order: a working agent is lifted to the top
   * of its section and drops back here when it stops. See `lib/rail.ts`.
   */
  railOrder: number;
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

/**
 * How a turn is paid for.
 *
 * `compatible` is an endpoint and a key the operator pasted. `chatgpt` is a
 * subscription signed in to on this machine, which has its own endpoint, its own
 * models and no per-call price. They are two providers rather than one with a
 * flag for the same reason the Rust side says so: almost nothing about a call is
 * the same between them.
 */
export type Provider = "compatible" | "chatgpt";

export interface Settings {
  /** What agents call you. Empty means they say "the operator". */
  operatorName: string;
  e2bKeySet: boolean;
  e2bKeyHint: string;
  computerIdleMinutes: number;
  kernelKeySet: boolean;
  kernelKeyHint: string;
  browserIdleMinutes: number;
  browserStealth: boolean;
  provider: Provider;
  baseUrl: string;
  /** The model used when a pasted key is paying. */
  defaultModel: string;
  /** The model used when a subscription is paying. Kept apart so switching
   *  providers does not overwrite either. */
  subscriptionModel: string;
  apiKeySet: boolean;
  apiKeyHint: string;
  requestTimeoutSecs: number;
  limits: GuardLimits;
  /** What a subscription can run, as the backend spells them. */
  subscriptionModels: string[];
}

/** Absent fields are left unchanged. An empty `apiKey` clears the key. */
export interface SettingsPatch {
  operatorName?: string;
  e2bApiKey?: string;
  computerIdleMinutes?: number;
  kernelApiKey?: string;
  browserIdleMinutes?: number;
  browserStealth?: boolean;
  provider?: Provider;
  baseUrl?: string;
  apiKey?: string;
  defaultModel?: string;
  subscriptionModel?: string;
  requestTimeoutSecs?: number;
  limits?: GuardLimits;
}

/**
 * Whether a ChatGPT subscription is signed in, and whose.
 *
 * No token, and no field one could arrive in: the webview never holds a
 * credential. The email is here because "signed in" alone does not tell an
 * operator whether they signed in to the account they meant to.
 */
export interface SubscriptionStatus {
  signedIn: boolean;
  email: string;
  /** As the service spells it: `plus`, `pro`, `team`, `enterprise`, `free`. */
  plan: string;
  /** A free plan signs in successfully and then cannot make one call. */
  includesCodex: boolean;
}

/**
 * Whether a Guaca account is signed in, and which service it is.
 *
 * No token, for the same reason as above. The origin is on it because in
 * development it is not `guaca.bot`, and an operator who cannot see which
 * service they linked to cannot tell the two apart.
 */
export interface AccountStatus {
  signedIn: boolean;
  email: string;
  origin: string;
}

/** One thing an authorized provider can do, as the service describes it. */
export interface AccountCapability {
  id: string;
  label: string;
  granted: boolean;
}

export interface AccountProvider {
  id: string;
  label: string;
  capabilities: AccountCapability[];
}

/**
 * What the account holds, as the service reports it.
 *
 * Read rather than kept. It changes when the operator authorizes something in
 * a browser rather than when this app does anything.
 */
export interface AccountConnectors {
  email: string;
  providers: AccountProvider[];
}

/** What the operator carries to a browser to finish signing in. */
export interface DeviceCode {
  verificationUrl: string;
  userCode: string;
  deviceAuthId: string;
  intervalSecs: number;
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
  /**
   * Part of the model's working, for as long as the turn lasts.
   *
   * Addressed to the placeholder and to nothing else: the agent it belongs to
   * is read from the stream it names, and it goes when that stream ends.
   */
  | { type: "reasoningDelta"; messageId: MessageId; text: string }
  /**
   * A tool call the turn has started, and then what came of it.
   *
   * Addressed to the placeholder for the same reason a thought is, and dropped
   * with it: the record of what a turn did is the message that lands at the end
   * of it, and these are only what that record looks like while it is still
   * being made. `callId` is the provider's own, which is what pairs the two.
   *
   * The finish carries the whole part rather than the outcome, so the chip
   * drawn while the turn runs and the chip drawn afterwards are one value read
   * once: a memory rewrite carries what it overwrote, and nothing outside the
   * runtime could supply it.
   */
  | {
      type: "toolStarted";
      messageId: MessageId;
      callId: string;
      name: string;
      arguments: unknown;
    }
  | { type: "toolFinished"; messageId: MessageId; callId: string; part: ToolCallPart }
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
  | { type: "approvalSettled"; approvalId: ApprovalId; state: ApprovalState }
  /**
   * One agent's schedule changed: it set a routine, edited one, cancelled one,
   * or one came due and moved. The list refetches; nothing here is patched,
   * because a schedule is a handful of rows.
   */
  | { type: "routinesChanged"; agentId: AgentId };

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
 * `once`, `daily`, `weekdays`, `weekly`, `monthly`, `every:<seconds>` for a
 * fixed gap, or `event:<service>/<topic>` for something happening in a
 * connected service. A string rather than a union of literals because both
 * `every:N` and `event:x/y` are open-ended, and because the next kind of
 * trigger should be a new value here rather than a new field. Read it with
 * `parseTrigger` in `lib/routine.ts`; nothing outside that file branches on
 * the raw text.
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
  /**
   * When it next fires, for a routine that waits on the clock.
   *
   * Null for one that does not: an event trigger fires when its event arrives
   * and holds no slot in the meantime. Anything drawing a countdown has to
   * answer for this case rather than render a date it invented.
   */
  nextRunAt: number | null;
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
  /**
   * What the firing bought, summed over its model calls.
   *
   * `calls: 0` is the one an operator needs: the routine was delivered and
   * nothing ran. Nothing else about the row tells that apart from a firing
   * that worked.
   */
  spent: Tokens;
}

/** Absent `inSecs` on an edit leaves the next firing where it was. */
export interface RoutineDraft {
  name: string;
  what: string;
  trigger: TriggerSpec;
  inSecs: number | null;
}

/**
 * What the transcript has to say about a query.
 *
 * Agents and groups are absent on purpose: this side is already holding both to
 * draw the rail, so they are matched here without a round trip. See
 * `lib/search.ts`, which puts the two halves in one list.
 */
export interface SearchHits {
  messages: MessageHit[];
  files: FileHit[];
  links: LinkHit[];
  routines: Routine[];
}

/** A matching message, with a window of its text rather than all of it. */
export interface MessageHit {
  id: MessageId;
  /** The channel to open to read it in context. */
  channelId: AgentId;
  from: Participant;
  to: Participant;
  excerpt: string;
  createdAt: number;
}

/** One attachment, and the message that carried it. Unique by digest. */
export interface FileHit {
  file: Attachment;
  messageId: MessageId;
  channelId: AgentId;
  from: Participant;
  createdAt: number;
}

/** A URL somebody wrote, and where they wrote it. */
export interface LinkHit {
  url: string;
  messageId: MessageId;
  channelId: AgentId;
  createdAt: number;
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

/**
 * Concatenated text of an envelope, matching the Rust `plain_text`.
 *
 * A fired routine's instruction counts, exactly as it does there: it is what
 * the model was sent, so the activity board naming what opened a run has to be
 * able to say it. Drawing a firing as a bubble is prevented by the transcript
 * choosing a row for the part, not by this hiding the words.
 */
export function plainText(envelope: Envelope): string {
  return envelope.parts
    .map((part) => (part.type === "text" ? part.text : part.type === "routine" ? part.what : null))
    .filter((text): text is string => text !== null)
    .join("\n")
    .trim();
}

export function isInterAgent(envelope: Envelope): boolean {
  return envelope.from.kind === "agent" && envelope.to.kind === "agent";
}
