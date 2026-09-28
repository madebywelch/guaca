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
/**
 * Whether an agent's browser asks before it acts in the operator's name.
 *
 * Per agent, and `open` by default: what the browser is signed in to is what
 * the operator handed over when they gave the agent the browser. The rule this
 * turns on cannot tell one account on a site from another, so it is a decision
 * about which agents are trusted rather than about which pages are safe.
 */
export type BrowserConsent = "open" | "askBeforeActing";
export type Trust = "operator" | "peer" | "system";
export type NoticeKind = "guardStop" | "upstreamError" | "interrupted" | "lifecycle";

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
   *
   * `artifact` is the kept page an `artifact` call wrote, at the version it
   * left it, and is what the transcript draws a card from. The id is assigned
   * by the write, so nothing in `arguments` could say which one a create made.
   */
  | {
      type: "toolCall";
      name: string;
      arguments: unknown;
      outcome: ToolOutcome;
      replaced?: string;
      artifact?: ArtifactMade;
    }
  | ({ type: "file" } & Attachment)
  /**
   * A routine coming due, drawn as one line the operator can open rather than
   * as dialogue. The instruction is in `what` and is what the model was sent;
   * `name` is the routine's name at the moment it fired, so a routine since
   * renamed does not rewrite what the transcript said it was.
   */
  | {
      type: "routine";
      routineId: RoutineId;
      name: string;
      what: string;
      /** What an event arrived with, when one did. Data, never instruction. */
      payload: string | null;
    }
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
    }
  /**
   * An agent asking the operator what, rather than whether. Its own part
   * because nothing answered here grants anything, which is the difference
   * every surface drawing the two has to make: see {@link Request}.
   */
  | {
      type: "question";
      id: ApprovalId;
      question: string;
      /** What the operator may pick. Empty is a written answer. */
      options: string[];
    };

/**
 * The one part with two lives: drawn as a chip while the turn is making the
 * call, and again out of the message that records it. Named because the live
 * half is carried whole by `toolFinished`, so both are built from one value.
 */
export type ToolCallPart = Extract<Part, { type: "toolCall" }>;

export type ApprovalId = string;

/** Something an agent may not do without being told it can. */
export type ProtectedAction = "createAgent" | "actOnBehalf" | "changeSettings";

/**
 * What an agent stopped its turn to put to the operator.
 *
 * The line between the two is what a yes does. A permission authorizes: the
 * agent could not do the thing, and the answer is what lets it. A question
 * informs: the agent could go either way and does not know which way is wanted,
 * so the answer is a value, and whatever it then does passes through the guards
 * it already had. That is why a question may draw the agent's own words on a
 * button and a permission may not.
 */
export type Request =
  | { kind: "permission"; action: ProtectedAction }
  | { kind: "question"; options: string[] };

/** What the operator can answer a permission with. A question takes text. */
export type Decision = "allow" | "alwaysAllow" | "deny";

export type ApprovalState = Decision | "pending" | "answered" | "expired";

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
  request: Request;
  summary: string;
  detail: DetailField[];
  state: ApprovalState;
  /** What the operator picked or wrote. Only ever set on a question. */
  answer: string | null;
  createdAt: number;
  decidedAt: number | null;
}

export type EscalationId = string;

/**
 * Work that has stopped, and that only the operator can move.
 *
 * Not an {@link Approval} and deliberately not a third {@link Request}: nothing
 * is parked on one, so there is no verdict, no value and no ten minute fuse.
 * The agent raised it on its way out of a turn and carried on without it.
 *
 * What makes a row worth more than the message it replaces is the pair at the
 * bottom. `raisedAt` never moves and `times` only goes up, so an agent that
 * hits the same wall on six turns is one row that says it has been true since
 * Tuesday and that six turns have gone into it.
 */
export interface Escalation {
  id: EscalationId;
  agentId: AgentId;
  groupId: GroupId;
  runId: RunId;
  /** The agent's own words, so every surface draws it as text. */
  summary: string;
  /** When it first went up. Never moved by a later raise. */
  raisedAt: number;
  /** When it was last restated, which is not the same question. */
  saidAt: number;
  /** How many turns have run into it. One when it goes up. */
  times: number;
  /** Set once the operator has taken it off the desk. */
  clearedAt: number | null;
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
export type ReasoningEffort =
  | "auto"
  | "none"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "max"
  | "ultra";

export interface SubscriptionModel {
  slug: string;
  defaultReasoningEffort: ReasoningEffort | null;
  reasoningEfforts: { effort: ReasoningEffort; description: string }[];
}

export interface InferenceOverrides {
  provider: Provider | null;
  baseUrl: string | null;
  /** The model used when a key is paying. */
  defaultModel: string | null;
  /** The model used when the subscription is paying. Two fields for the reason
   *  the app keeps two: the providers have disjoint model names. */
  subscriptionModel: string | null;
  reasoningEffort: ReasoningEffort | null;
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
 * Where a skill lives, which is who may write it. Guaca's own are read-only,
 * the operator's are read by every crew, and a crew's own can be written by
 * its agents as well as by the operator.
 */
export type SkillScope =
  | { kind: "bundled" }
  | { kind: "workspace" }
  | { kind: "crew"; groupId: GroupId };

/** A markdown document of instructions for one kind of task. */
export interface Skill {
  name: string;
  description: string;
  scope: SkillScope;
  /** Empty in a listing; read when the skill is opened. */
  body: string;
  updatedAt: number;
  /** What sits beside its `SKILL.md`, as relative paths. */
  files: string[];
  /** The page it was added from, when it came from somewhere. */
  origin?: string | null;
}

/** Which of skills.sh's three rankings. */
export type DirectoryBoard = "allTime" | "trending" | "hot";

/** One skill on skills.sh, without its files. */
export interface DirectoryListing {
  /** `{source}/{slug}`, and its path on skills.sh. */
  id: string;
  source: string;
  slug: string;
  name: string;
  installs: number;
  /** Its page on skills.sh. */
  url: string;
  /** Whether skills.sh keeps a copy Guaca can add: a GitHub repository's. */
  addable: boolean;
}

export interface DirectoryPage {
  skills: DirectoryListing[];
  page: number;
  hasMore: boolean;
}

export interface SkillAudit {
  provider: string;
  verdict: "pass" | "warn" | "fail" | "unknown";
  summary: string;
}

/** A skill on skills.sh, opened: what the operator reads before adding it. */
export interface DirectorySkill {
  id: string;
  /** The name it will have here. */
  name: string;
  description: string;
  body: string;
  files: string[];
  /** Binds what was read to what gets added. */
  hash: string;
  url: string;
  /** `null` when the audits could not be read, `[]` when none has run. */
  audits: SkillAudit[] | null;
}

export interface SkillDraft {
  name: string;
  description: string;
  body: string;
  /** The name it had, when the edit renames it. */
  previous?: string;
}

/** What is open over the channel, as the host is told it. */
export type Overlay =
  | "settings"
  | "crewSettings"
  | "calendar"
  | "artifacts"
  | "forYou"
  | "search"
  | "cafeteria"
  | "agentEditor";

/**
 * What the operator is looking at, reported by the window with focus so an
 * agent asked about "this pane" can read what it is.
 */
export interface OperatorView {
  agentId: AgentId | null;
  overlay: Overlay | null;
  /** The pane inside the overlay, as the app names it: `limits`, `skills`. */
  section: string | null;
  groupId: GroupId | null;
}

/** One of Guaca's own tools, described from what the agent is told. */
export interface ToolSummary {
  name: string;
  summary: string;
  /** What an agent has to be given before it is offered this one. */
  needs: string | null;
}

/** One file in an agent's notebook, as a listing shows it. */
export interface NotebookEntry {
  path: string;
  chars: number;
  updatedAt: number;
}

/** One key per scope, for the counter a list redraws on. */
export function skillScopeKey(scope: SkillScope): string {
  return scope.kind === "crew" ? `crew:${scope.groupId}` : scope.kind;
}

/**
 * A credential granted to selected agents in one group.
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
  agents: AgentId[];
  /** Empty for wire compatibility; no part of a secret is returned. */
  secretHint: string;
  createdAt: number;
  updatedAt: number;
}

/**
 * Write-only creation input. Updates may replace the value and agent grants.
 */
export interface ConnectorDraft {
  groupId: GroupId;
  service: string;
  account: string;
  envVar: string;
  note: string;
  secret: string;
  agents?: AgentId[];
}

/**
 * Which program writes the code when an agent calls `code`.
 *
 * Three, because a subscription is spent by the program it was issued to: a
 * Claude plan pays for work done by `claude`, a ChatGPT plan for work done by
 * `codex`, and `pi` spends whatever it is signed in to, which is how an
 * OpenRouter key writes code. An operator whose one plan is spent needs another
 * program, not a different setting on the same one.
 */
export type Harness = "pi" | "claude" | "codex";

/** What an operator is shown for each, and what the panel offers in order. */
export const HARNESSES: { readonly id: Harness; readonly label: string }[] = [
  { id: "codex", label: "Codex" },
  { id: "claude", label: "Claude Code" },
  { id: "pi", label: "pi" },
];

/**
 * Whether an agent's pushes stop on the operator's desk first.
 *
 * A push, a pull request, a merge or a release is the operator's own name going
 * somewhere git cannot take it back from. Everything else is what the terminal
 * and git's undo already cover, and is never gated. It holds for every door the
 * agent has: its own shell and its coding job alike.
 *
 * `open` is the default because the prompt every job is given says nobody will
 * answer a question: an operator turning this on is an operator saying they
 * will be there.
 */
export type Gate = "open" | "askBeforePushing";

/**
 * The coding session an agent last ran, which a follow-up carries on.
 *
 * Only ever continued by the program that wrote it: the id is the harness's
 * own, and means nothing to the other two.
 */
export interface CodingSession {
  harness: Harness;
  id: string;
  /** Where it ran, relative to the agent's terminal. `.` is the terminal itself. */
  directory: string;
  updatedAt: number;
}

/** Where an agent's terminal is, on the machine Guaca runs on, and what it last ran there. */
export interface TerminalView {
  path: string;
  session: CodingSession | null;
  /** The shell line that opens that session on the host. Spelled in Rust, once. */
  resume: string | null;
  /** What was chosen inside each of the three harnesses. */
  tunings: HarnessTuning[];
  guacaKey: GuacaKey;
}

/**
 * What a message to a coding job came to: a correction into the one running,
 * or a new turn in the session the last one left.
 */
export type Continued = { kind: "steered" } | { kind: "resumed"; directory: string };

/**
 * One harness, as the machine reports it.
 *
 * The install command comes from Rust rather than being spelled here, because
 * it is the same string a refused job quotes at an agent, and two copies of an
 * operational fact drift the day a vendor renames a package.
 */
export interface HarnessOnMachine {
  harness: Harness;
  installed: boolean;
  /** What it says its version is, or empty when it is not there. */
  version: string;
  /**
   * Whether a job on it can be reached while it runs.
   *
   * False for a program older than the one the behavior was measured against.
   * An old Claude Code still runs a job, which can be stopped but not steered.
   * An old `pi` is refused with the update command, because its protocol is
   * how it is driven at all.
   */
  bridged: boolean;
  install: string;
  /**
   * Why this workspace will not run it, when it will not. Drawn as the row's
   * reason rather than by dropping the row, because a harness that silently
   * vanishes is a panel that explains nothing.
   */
  withheld?: string | null;
  signedIn?: boolean | null;
  signIn?: string;
  /** Every effort word the program takes. A model may take fewer. */
  efforts: string[];
}

/**
 * Which account pays for a harness's model calls. `guacaKey` is only pi's:
 * Guaca's own key, lent through a loopback relay that keeps it.
 */
export type Payer = "own" | "guacaKey";

/**
 * What the operator chose inside one harness for one agent. Null is the
 * program's own setting.
 */
export interface Tuning {
  model: string | null;
  effort: string | null;
  pays: Payer;
}

export interface HarnessTuning extends Tuning {
  harness: Harness;
}

/** Whether Guaca has a key to lend pi, and where it is spent. Never the key. */
export interface GuacaKey {
  set: boolean;
  endpoint: string;
  /** OpenRouter's models are the ones pi can list for it. */
  openrouter: boolean;
  /** What a job runs when no model was chosen: the key's own. */
  defaultModel: string;
}

/** One model a harness offers, as its own picker shows it. */
export interface ModelOffer {
  /** What `--model` takes. */
  id: string;
  label: string;
  detail: string;
  /** Whether it runs when nothing is chosen. */
  default: boolean;
  /** The effort words this model takes. Empty when it takes none. */
  efforts: string[];
}

export type PluginId = string;

/**
 * The servers Guaca ships the address of.
 *
 * Closed, and the same everywhere: each of these is on the list because
 * somebody checked that it publishes its own tools, acts on the operator's
 * account and lets an application register itself.
 */
export type CatalogKind =
  | "neon"
  | "cloudflare"
  | "linear"
  | "stripe"
  | "agentmail"
  | "supermemory"
  | "firecrawl"
  | "google";

/**
 * What a connected plugin is called, which is also the prefix its tools are
 * called by.
 *
 * One of the eight, or the name an operator gave a server they added. Not a union
 * of the eight, because the whole point of a custom server is that this side does
 * not know the set: what a plugin is called comes back from Rust with the row,
 * along with a `custom` flag saying whether anybody vouched for it.
 */
export type PluginKind = CatalogKind | (string & {});

/** A plugin on offer, before anybody has connected it. */
export interface PluginOffer {
  kind: CatalogKind;
  name: string;
  /** One line about what the crew gets. */
  blurb: string;
  docs: string;
  /** Where the sign-in and every later call goes, shown before it is clicked. */
  endpoint: string;
  /**
   * Whether this one's credential is the operator's Guaca account, and so
   * whether there is an identity to choose before connecting.
   */
  accountBacked: boolean;
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
 * One of a connected plugin's tools, and who may call it.
 *
 * The description and not the schema: an operator deciding whether the crew may
 * call `delete_customer` needs the sentence the vendor wrote about it and has
 * no use for the shape of its arguments.
 *
 * `access` is the same answer the plugin itself takes, one level down, and it
 * is `everyone` until somebody says otherwise — what the store writes down is
 * the narrowing, so a tool a vendor ships next month arrives on rather than
 * invisible. `chosen` with no agents is a tool switched off for the crew, which
 * is the only state the old two-way switch could express.
 *
 * The two compose rather than overlap. The plugin's answer is who may spend the
 * sign-in; this one is who may do that particular thing with it. Two agents on
 * one inbox where one reads and the other sends needs both.
 */
export interface PluginToolCard {
  /** The server's own name for it. Prefixed with the plugin, a model calls it. */
  name: string;
  description: string;
  access: PluginAccess;
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
  /**
   * What it is called, and where it is.
   *
   * Read off the row rather than paired with a catalog entry, because a server
   * the operator added has no catalog entry: a connected plugin has to be
   * drawable from what came back, or a row the panel cannot name is a row with
   * no way to disconnect it.
   */
  name: string;
  endpoint: string;
  /** Whether nobody vouched for this server: the operator added it. */
  custom: boolean;
  /** Whose account, when the server said. Usually blank. */
  account: string;
  /**
   * Every tool the server published, switched off ones included: a list that
   * left them out would be a panel with no way to switch one back on.
   */
  tools: PluginToolCard[];
  /** Which of the crew is offered them. `everyone` until somebody says else. */
  access: PluginAccess;
  /** Which authorized identity at the Guaca account this crew uses, if any. */
  connection: string;
  /**
   * Which headers the operator gave this server, by name and never by value.
   *
   * Drawn so the panel can say what is being sent — an operator debugging their
   * own server needs to know whether `x-api-key` is on the request — without
   * being a place a credential can be read back out of. Empty for every server
   * on the catalog and for most added ones.
   */
  headers: string[];
  signedIn: boolean;
  connectedAt: number;
}

/**
 * One header the operator wrote, on its way to Rust.
 *
 * The value only ever travels in this direction. What comes back on a `Plugin`
 * is the names, for the reason a connector's secret never comes back either.
 */
export interface HeaderPair {
  name: string;
  value: string;
}

/**
 * What one dial of a server found out, without connecting anything.
 *
 * The answer to "why does my server not work", which is otherwise a single
 * sentence out of whichever layer failed first. The fields are separate because
 * the failures are: a 405 on the current transport and a working event stream
 * are the same sentence and opposite instructions to a person.
 */
export interface ServerReport {
  /** The address as Rust canonicalized it, which is what would be stored. */
  endpoint: string;
  /** Which transport it answered on. Blank when nothing was established. */
  transport: string;
  /** The revision the two of them settled on. Blank for the same reason. */
  protocol: string;
  /** Whether it wanted a handshake. Not the same question as the transport. */
  handshake: boolean;
  signin: SigninNeed;
  /** How the server names itself, when it says. Often blank. */
  server: string;
  /** Every tool it published. Empty for a server that wants a sign-in. */
  tools: string[];
  ms: number;
}

/**
 * What the server did about a credential.
 *
 * `wanted` and `refused` are the same status code on the wire and opposite
 * problems: one is a server that signs in, the other is a key it will not take.
 */
export type SigninNeed = "none" | "wanted" | "accepted" | "refused";

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
  /** The host, normalized: `linkedin.com`. */
  domain: string;
  /** A recognized service's real name, or the domain when it is a guess. */
  service: string;
  /** False when this came from the weaker visited-plus-session-cookie rule. */
  recognized: boolean;
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
  /**
   * Whether that browser stops and asks before it acts in the operator's name.
   *
   * `open` unless the operator held this agent back, because giving it the
   * browser was already the decision about what the browser is signed in to.
   * Held back, a press or a typed line on a site the browser holds a session
   * for, in a turn that has read a page, parks the turn and asks.
   */
  browserConsent: BrowserConsent;
  /**
   * Whether this agent may send errands: its own model, called again with one
   * brief and its tools, up to three at once. Off unless the operator says so.
   */
  runsErrands: boolean;
  /**
   * Whether the operator has given this agent a terminal: a directory of its
   * own on the machine Guaca runs on, the shell that starts there, and a
   * coding harness. A decision, like `hasComputer`: taking it away keeps the
   * directory and the two answers below.
   */
  hasTerminal: boolean;
  /** Which program writes this agent's code. */
  harness: Harness;
  /** Whether this agent's pushes ask the operator first. */
  gate: Gate;
  name: string;
  avatar: string;
  color: string;
  model: string;
  reasoningEffort?: ReasoningEffort | null;
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
  /**
   * When this agent was thrown out, while it can still be pulled back.
   *
   * Set only on a terminated row, and only while the wait lasts: `null` is both
   * an agent nobody deleted and one whose thirty days are up and whose memory,
   * machines and schedule are already gone. See `lib/compost.ts`.
   */
  discardedAt: number | null;
}

export interface AgentDraft {
  /** Omitted means "leave it where it is" on update, "default group" on create. */
  groupId?: GroupId;
  name: string;
  avatar: string;
  color: string;
  model: string;
  reasoningEffort?: ReasoningEffort | null;
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
 * models and no per-call price. `claude` is not an endpoint at all: it runs the
 * `claude` program once per model call, which is the only way an Anthropic
 * subscription can pay for a turn, and it takes its sign-in and its model from
 * that program rather than from anything set here. They are three providers
 * rather than one with a flag for the same reason the Rust side says so: almost
 * nothing about a call is the same between them.
 */
export type Provider = "compatible" | "chatgpt" | "claude";

/**
 * One model OpenRouter ranks for a kind of work, as a suggestion beside a model
 * field.
 *
 * Ranked by capability inside the pool of models that use case actually gets
 * sent, which is not the order the endpoint returns by default: `catalog.rs`
 * has the argument. The price is here because that ranking ignores it, and the
 * most capable model in a pool is regularly the dearest thing in it.
 */
export interface RankedModel {
  /** The slug, which is what the model field holds. */
  id: string;
  /** How the vendor writes it for a person. */
  name: string;
  contextLength: number;
  /** Dollars per million prompt tokens. `null` when none was quoted, which is
   *  not the same as free. */
  promptPerMillion: number | null;
  completionPerMillion: number | null;
}

/**
 * What this workspace can do, which is a property of where it runs.
 *
 * Five flags, and every one of them is something physically on the operator's
 * machine rather than a feature nobody finished. A local workspace has all
 * five; one running on a server has none, because a credential bound to a
 * program, a working tree with uncommitted work in it, a model server on
 * loopback and a file on a disk cannot be reached from an origin.
 *
 * Read once when the window opens. A deployment cannot change under a running
 * app, so this is a constant that happens to arrive over IPC.
 *
 * The panels gate on it and the commands refuse anyway. That is not belt and
 * braces: a webview on a stale bundle still draws the control it was built
 * with, and the refusal is what turns that into a sentence instead of a
 * failure.
 */
export interface Capabilities {
  /** Whether inference may point at a model server on loopback. */
  loopbackEndpoints: boolean;
  /** Whether a turn may be paid for by a Claude plan. */
  claudeProvider: boolean;
  /** Whether Claude Code may be the harness that writes the code. */
  claudeCodeHarness: boolean;
  /** Whether a file may be named by a path on the operator's own disk. */
  localFiles: boolean;
}

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
  reasoningEffort: ReasoningEffort;
  apiKeySet: boolean;
  apiKeyHint: string;
  requestTimeoutSecs: number;
  limits: GuardLimits;
  /** Initial choices while the signed-in account's model catalog loads. */
  subscriptionModels: string[];
  /** The status bar's buttons. Absent from a host older than them. */
  quickActions?: QuickAction[];
  /** Kept pages pinned to the status bar. Absent from a host older than them. */
  widgets?: Widget[];
}

/** How much of the status bar a pinned page takes. */
export type WidgetWidth = "narrow" | "wide";

/**
 * A kept page on the status bar, drawn from its condensed view. An agent's is
 * only there because the operator approved it, reads included.
 */
export interface Widget {
  artifactId: ArtifactId;
  width: WidgetWidth;
  /** Minutes between the page's reads while it is on the bar. */
  everyMinutes: number;
  /** "the operator", or the agent that asked for it. */
  addedBy: string;
}

/** Somewhere a quick action can open. */
export type QuickPlace =
  | { kind: "channel"; agentId: AgentId }
  | { kind: "calendar" }
  | { kind: "forYou" }
  | { kind: "settings"; section?: string | null }
  | { kind: "crewSettings"; groupId: GroupId };

/** What pressing a quick action does: a closed set the app draws and runs. */
export type QuickDoes =
  | { kind: "message"; agentId: AgentId; text: string }
  | { kind: "open"; place: QuickPlace };

/**
 * One button on the status bar. An agent's is only there because the operator
 * approved it, with the whole message shown, since a message it sends is sent
 * as the operator.
 */
export interface QuickAction {
  id: string;
  label: string;
  does: QuickDoes;
  /** "the operator", or the agent that asked for it. */
  addedBy: string;
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
  reasoningEffort?: ReasoningEffort;
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
/**
 * One identity the operator has authorized at a provider.
 *
 * A person can authorize the same provider twice — a work Google and a personal
 * one — and each is its own grant. A group binds to one of these, which is what
 * lets two crews use two mailboxes.
 */
export interface AccountConnection {
  id: string;
  provider: string;
  /** The provider's own name for it, which is how two are told apart. */
  label: string;
  capabilities: string[];
}

export interface AccountConnectors {
  email: string;
  providers: AccountProvider[];
  connections: AccountConnection[];
}

/** What the operator carries to a browser to finish signing in. */
export interface DeviceCode {
  verificationUrl: string;
  userCode: string;
  deviceAuthId: string;
  intervalSecs: number;
}

/**
 * Where the menu bar is asking the window to go.
 *
 * One gesture, two destinations, and neither is the other's fallback: an agent
 * is `select`, which follows it into whatever crew it is in, and a crew is
 * `focusGroup`, which opens the crew and chooses nobody in it. Kept in step
 * with `Reveal` in `tray.rs`, which `ipc.contract.test.ts` checks.
 */
export type Reveal =
  | { kind: "forYou" }
  | { kind: "agent"; id: AgentId }
  | { kind: "crew"; id: GroupId };

export type UiEvent =
  | {
      type: "liveSnapshot";
      activity: Record<AgentId, Activity>;
      streams: Record<
        MessageId,
        { channelId: AgentId; agentId: AgentId; runId: RunId; to: Participant; text: string }
      >;
      /** Which agents have a coding job running, and the directory each is in. */
      building: Record<AgentId, string>;
    }
  | { type: "agentsChanged" }
  /** `client` is the page whose command asked; absent from an older host. */
  | { type: "openUrl"; url: string; client?: string | null }
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
   * drawn while the turn runs and the chip drawn afterward are one value read
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
  | { type: "escalationRaised"; escalationId: EscalationId; agentId: AgentId }
  | { type: "decisionsChanged" }
  | { type: "decisionReminder"; count: number }
  | { type: "escalationCleared"; escalationId: EscalationId }
  /**
   * One agent's schedule changed: it set a routine, edited one, canceled one,
   * or one came due and moved. The list refetches; nothing here is patched,
   * because a schedule is a handful of rows.
   */
  | { type: "routinesChanged"; agentId: AgentId }
  /**
   * One agent rewrote its own memory. The panel drawing it reads the file
   * again; nothing here is patched, because the file is a page at most and the
   * event carries none of it.
   */
  | { type: "memoryChanged"; agentId: AgentId }
  /**
   * One agent appended a working note. Its own event rather than a second
   * meaning for `memoryChanged`: notes are written far more often, and folding
   * them together would have every note refetch a memory that has not moved.
   */
  | { type: "workingNotesChanged"; agentId: AgentId }
  /**
   * The workspace settings changed, from any client or from an agent. Carries
   * the redacted settings, which every open client needs and nothing more.
   */
  | { type: "settingsChanged"; settings: Settings }
  /** One agent changed a file in its own notebook. */
  | { type: "notebookChanged"; agentId: AgentId }
  /** A skill was written or deleted, by the operator or by an agent. */
  | { type: "skillsChanged"; scope: SkillScope }
  /**
   * A coding job could not run, for a reason only the operator can fix.
   *
   * The agent that asked is told in its own channel, and that is not enough: a
   * sentence inside one transcript is a sentence nobody reads. This is the same
   * thing said where somebody setting up is looking.
   */
  | {
      type: "codingJobFailed";
      agentId: AgentId;
      directory: string;
      harness: string;
      reason: string;
    }
  /**
   * Something moved on one crew's calendar.
   *
   * The crew rather than the agent, because the calendar is the crew's and the
   * panel drawing it is usually showing every crew at once.
   */
  | { type: "calendarChanged"; groupId: GroupId }
  /**
   * One of a crew's artifacts was made, changed or changed hands, by an agent
   * or by the operator. The crew, for the reason the calendar's is.
   */
  | { type: "artifactsChanged"; groupId: GroupId }
  /**
   * A coding job started or ended in an agent's terminal.
   *
   * The rail draws this because nothing else would: `code` returns as soon as
   * the harness is up and the turn ends, so an agent sits idle while a coding
   * agent works for it for twenty minutes, and the crew reads as stopped at
   * exactly the moment it is building.
   */
  | { type: "codingJobStarted"; agentId: AgentId; directory: string }
  | { type: "codingJobFinished"; agentId: AgentId }
  /**
   * One line of what a running coding job is doing.
   *
   * Ephemeral, exactly as a turn's thinking is: held while the job runs and
   * dropped when it ends. The record of what a job did is the message it
   * delivers at the end; this is what that looks like before it exists.
   */
  | {
      type: "codingProgress";
      agentId: AgentId;
      /** A tool name, or empty when the coding agent is talking. */
      tool: string;
      /** The command, the path, or the sentence. */
      detail: string;
    };

/** One line of a running coding job, as the panel draws it. */
export interface CodingLine {
  tool: string;
  detail: string;
}

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
  /** Working notes dropped, which is the other store and a row count. */
  workingNotes: number;
  calls: number;
}

export type OccasionId = string;

/**
 * One thing on a crew's calendar.
 *
 * Guaca's own, not a view of Google's. What is on it is what an agent or the
 * operator wrote down as coming: meetings, deadlines, filings, renewals. It
 * fires nothing and wakes nobody, which is the whole of what separates it from
 * a {@link Routine}.
 */
export interface Occasion {
  id: OccasionId;
  /**
   * The crew whose calendar this is on, and the wall. An agent can only reach
   * occasions in its own crew; the operator's view is the one read that crosses
   * every crew at once.
   */
  groupId: GroupId;
  /**
   * The agent that put it there. `null` is the operator's own, and stays `null`
   * once written: an occasion is the crew's rather than the agent's.
   */
  agentId: AgentId | null;
  title: string;
  /** What is worth knowing to walk into it prepared. Often empty. */
  detail: string;
  /** A room, a city, a link. Often empty. */
  place: string;
  /** When it starts. Local midnight of the day when {@link Occasion.allDay}. */
  startsAt: number;
  /**
   * How long it runs. `null` is a moment with no stated end, which most
   * deadlines are, and is always `null` on an all-day occasion.
   */
  minutes: number | null;
  /**
   * A day with no time on it: a deadline, a filing, a birthday. Distinct from
   * midnight, which is a time somebody chose, and the reason nothing may draw
   * `startsAt` as a clock without checking this first.
   */
  allDay: boolean;
  createdAt: number;
  updatedAt: number;
}

/**
 * What the operator can set on one.
 *
 * `startsAt` is the string that was typed, parsed in Rust, which is what lets
 * one field mean both `2026-09-14` for a whole day and `2026-09-14 15:00` for a
 * time. A second field saying which it was would be a field the two writers
 * could disagree on.
 */
export interface OccasionDraft {
  groupId: GroupId;
  title: string;
  detail: string;
  place: string;
  startsAt: string;
  minutes: number | null;
}

export type ArtifactId = string;

/**
 * Who did something to an artifact. An agent is named as it was at the time,
 * because the log is a record of what happened and has to keep reading after
 * the agent is renamed, moved or deleted.
 */
export type ArtifactActor = { kind: "operator" } | { kind: "agent"; id: AgentId; name: string };

/**
 * A page a crew keeps, as the list shows it. The page itself is read on its
 * own, a version at a time: `api.artifactPage`.
 */
export interface Artifact {
  id: ArtifactId;
  /** The crew it belongs to, and the wall: an agent reaches only its own crew's. */
  groupId: GroupId;
  /**
   * Who answers for it. `gone` is an owner deleted or moved to another crew,
   * still named until somebody takes it over: ownership moves by a decision,
   * never by itself. `null` only once the owning agent's row is gone for good.
   */
  owner: { id: AgentId; name: string; gone: boolean } | null;
  title: string;
  /** Starts at 1 and only goes up. A restore is a new version, not a rewind. */
  version: number;
  /** Who made the current version. */
  editedBy: ArtifactActor;
  createdAt: number;
  /** When the current version was made. Ownership changes do not move it. */
  updatedAt: number;
  /** The connector reads the current version declares. Empty for most pages. */
  sources: ArtifactSource[];
  /** Whether the operator allowed exactly these reads. Never true for none. */
  sourcesAllowed: boolean;
  /**
   * Whether the current version has a condensed view: the one-row document the
   * status bar draws, and so what a page needs before it can be pinned.
   */
  condensed: boolean;
}

/** What the status bar draws for a pinned page. */
export interface CondensedView {
  artifact: Artifact;
  /** `null` for a version without one, which a restore can put back. */
  page: string | null;
}

/**
 * One read a page declares: a connector tool and exactly what to send it,
 * fixed when the page was written. The page cannot change the arguments, which
 * is what keeps a page that reads from being a page that can send.
 */
export interface ArtifactSource {
  /** The key the page reads the result under. */
  name: string;
  /** A connector tool as an agent calls it: `linear__list_issues`. */
  tool: string;
  arguments: unknown;
}

/** What one read came back with, as the page is handed it. */
export interface ArtifactRead {
  name: string;
  /** JSON, when the tool answered with some. */
  data: unknown;
  text: string | null;
  /** Why there is neither, in words the page can show. */
  error: string | null;
}

export type ArtifactChange = "created" | "edited" | "restored" | "took" | "handed" | "allowed";

/** One row of an artifact's log. */
export interface ArtifactEntry {
  seq: number;
  at: number;
  change: ArtifactChange;
  by: ArtifactActor;
  /** The version once this was done. Ownership changes leave it where it was. */
  version: number;
  /** Who owned it once this was done, by the name they had then. */
  owner: string | null;
  note: string;
}

export interface ArtifactDetail {
  artifact: Artifact;
  /** Oldest first, and without any page in it. */
  log: ArtifactEntry[];
}

/** What an `artifact` call wrote, as its tool call records it. */
export interface ArtifactMade {
  id: ArtifactId;
  version: number;
  /** The title at that moment, as a routine's chip keeps its name. */
  title: string;
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
   * Whether a firing that comes due while the agent is already working is
   * dropped rather than queued behind what it is doing.
   *
   * Only ever true on a routine that repeats: skipping moves the slot on, and
   * the slot a one-off holds is the only one it has.
   */
  skipIfWorking: boolean;
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

/**
 * What happened at one firing. A test is the operator's button, not the clock;
 * a skip is a firing the routine dropped because the agent was already working;
 * an event is a post to the receiver, which moved nothing on the clock.
 */
export type RunKind = "scheduled" | "test" | "skipped" | "event";

/**
 * One firing. `runId` threads back to everything it produced: the messages in
 * the channel and the model calls on the bill are both filed under it.
 *
 * Null on a skip, which produced neither. An id there would read back exactly
 * like a delivery that spent nothing, and those are the two this row exists to
 * tell apart.
 */
export interface RoutineRun {
  runId: RunId | null;
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

/**
 * Where an event is posted to fire a routine, and what the post carries.
 *
 * A port of zero is a receiver that is not running, and the panel says so
 * rather than printing an address nothing answers.
 */
export interface WebhookAddress {
  port: number;
  url?: string | null;
  secret: string;
}

/** Absent `inSecs` on an edit leaves the next firing where it was. */
export interface RoutineDraft {
  name: string;
  what: string;
  trigger: TriggerSpec;
  inSecs: number | null;
  /** Refused on a trigger that does not repeat, so the panel never sends it. */
  skipIfWorking: boolean;
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
  /** Kept pages whose title matched, every crew's. */
  artifacts: Artifact[];
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

/**
 * What the menu bar draws, as the window hands it over.
 *
 * Mirrors `menubar::Presence` in Rust. The strip reads this machine's runtime
 * when the window shows this machine's workspace, and reads this when the
 * window shows a box: the window is the one thing holding the box's state.
 */
export interface Presence {
  roster: Record<AgentId, { name: string; crew: GroupId }>;
  crews: { id: GroupId; name: string }[];
  activity: Record<AgentId, Activity>;
  waiting: Approval[];
  stuck: Escalation[];
  decisions: WorkDecision[];
  /** Spent since this window opened. */
  session: Tokens;
  allTime: Tokens;
  running: number;
}

/** A click on a menu bar row drawn from a box, handed back to the window. */
export type MenubarAsk =
  | { kind: "stopAll" }
  | { kind: "decide"; approval: ApprovalId; decision: Decision };

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
 * the model was sent, so the flow board naming what opened a run has to be
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

/**
 * One line an agent wrote about where its work stands.
 *
 * The counterpart to memory and shaped by what memory is not: appended rather
 * than rewritten, stamped, and dropped by age rather than by anybody's
 * decision. `at` is milliseconds since the epoch, as everything else here is.
 */
export interface WorkingNote {
  at: number;
  body: string;
}

export interface WorkDecision {
  id: string;
  agentId: AgentId;
  groupId: GroupId;
  topic: string;
  request: {
    question: string;
    context: string;
    recommendation: string;
    options: string[];
    source: string;
  };
  status: "pending" | "answered" | "completed" | "withdrawn";
  answer: string | null;
  outcome: string | null;
  createdAt: number;
  updatedAt: number;
  dueAt: number | null;
  remindAt: number;
  snoozedUntil: number | null;
  deliveryRun: RunId | null;
  interrupted: boolean;
}
