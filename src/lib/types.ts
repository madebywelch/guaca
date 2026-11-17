export type AgentId = string;
export type GroupId = string;
export type MessageId = string;
export type RunId = string;

export type PermissionClass = "actOnBehalf" | "createAgent" | "changeSettings" | "actOnSite";
export type PermissionField = "recipients" | "domain" | "amount" | "agentName" | "settingKeys";
export type PermissionPredicate =
  | { op: "is" | "under" | "outsideDomain"; value: string }
  | { op: "oneOf"; values: string[] }
  | { op: "atMost" | "greaterThan"; value: string; currency: string };
export interface PermissionCondition {
  field: PermissionField;
  predicate: PermissionPredicate;
}
export interface PermissionRuleDraft {
  groupId: GroupId | null;
  effect: "allow" | "block";
  class: PermissionClass;
  tool: string | null;
  agents: AgentId[];
  conditions: PermissionCondition[];
}
export interface PermissionRule extends PermissionRuleDraft {
  id: string;
  version: number;
  wrote: string;
  createdAt: number;
}

export type Lifecycle = "active" | "paused" | "terminated";
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
  | { status: "partial"; summary: string; refused: RefusedRecipient[] }
  | { status: "refused"; reason: string }
  | { status: "failed"; error: string };

export type Intent = "work" | "courtesy";

export interface Attachment {
  digest: string;
  name: string;
  mime: string;
  bytes: number;
}

export interface Staged {
  attached: Attachment[];
  refused: string[];
}

export type Part =
  | { type: "text"; text: string }
  | { type: "json"; name: string; value: unknown }
  | { type: "notice"; kind: NoticeKind; text: string }
  | {
      type: "toolCall";
      name: string;
      arguments: unknown;
      outcome: ToolOutcome;
      replaced?: string;
      artifact?: ArtifactMade;
      ruled?: string;
    }
  | ({ type: "file" } & Attachment)
  | {
      type: "routine";
      routineId: RoutineId;
      name: string;
      what: string;
      payload: string | null;
    }
  | {
      type: "approval";
      id: ApprovalId;
      action: ProtectedAction;
      summary: string;
      detail: DetailField[];
    }
  | {
      type: "question";
      id: ApprovalId;
      question: string;
      options: string[];
    }
  | {
      type: "decision";
      id: string;
      request: DecisionRequest;
      answer: string;
      resumed: boolean;
    }
  | { type: "tidy"; reported: string[]; notes: string[] };

export type ToolCallPart = Extract<Part, { type: "toolCall" }>;

export type ApprovalId = string;

export type ProtectedAction = "createAgent" | "actOnBehalf" | "changeSettings";

export type Request =
  | { kind: "permission"; action: ProtectedAction }
  | { kind: "question"; options: string[] };

export type Decision = "allow" | "alwaysAllow" | "deny";

export type ApprovalState = Decision | "pending" | "answered" | "expired";

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
  answer: string | null;
  createdAt: number;
  decidedAt: number | null;
  lapsesAt: number | null;
}

export type EscalationId = string;

export interface Escalation {
  id: EscalationId;
  agentId: AgentId;
  groupId: GroupId;
  runId: RunId;
  summary: string;
  raisedAt: number;
  saidAt: number;
  times: number;
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
  intent: Intent;
  cause: MessageId | null;
  createdAt: number;
}

export interface Group {
  id: GroupId;
  name: string;
  agentCount: number;
  createdAt: number;
  inference: InferenceOverrides;
  apiKeySet: boolean;
  apiKeyHint: string;
  limits: GroupLimits;
  tidy: GroupTidy;
}

export interface GroupTidy {
  on: boolean;
  hour: number;
}

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
  defaultModel: string | null;
  subscriptionModel: string | null;
  reasoningEffort: ReasoningEffort | null;
  requestTimeoutSecs: number | null;
}

export interface GroupLimits {
  maxHops: number | null;
  maxStepsPerRun: number | null;
  maxFanoutPerCall: number | null;
  maxSendsPerPair: number | null;
  maxToolRounds: number | null;
}

export interface GroupDraft {
  name: string;
  inference?: InferenceOverrides;
  apiKey?: string;
  limits?: GroupLimits;
  tidy?: GroupTidy;
}

export type ConnectorId = string;

export type SkillScope =
  | { kind: "bundled" }
  | { kind: "workspace" }
  | { kind: "crew"; groupId: GroupId };

export interface Skill {
  name: string;
  description: string;
  scope: SkillScope;
  body: string;
  updatedAt: number;
  files: string[];
  origin?: string | null;
}

export type DirectoryBoard = "allTime" | "trending" | "hot";

export interface DirectoryListing {
  id: string;
  source: string;
  slug: string;
  name: string;
  installs: number;
  url: string;
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

export interface DirectorySkill {
  id: string;
  name: string;
  description: string;
  body: string;
  files: string[];
  hash: string;
  url: string;
  audits: SkillAudit[] | null;
}

export interface SkillDraft {
  name: string;
  description: string;
  body: string;
  previous?: string;
}

export type Overlay =
  | "settings"
  | "crewSettings"
  | "calendar"
  | "artifacts"
  | "forYou"
  | "search"
  | "cafeteria"
  | "agentEditor";

export interface OperatorView {
  agentId: AgentId | null;
  overlay: Overlay | null;
  section: string | null;
  groupId: GroupId | null;
}

export interface ToolSummary {
  name: string;
  summary: string;
  needs: string | null;
}

export interface NotebookEntry {
  path: string;
  chars: number;
  updatedAt: number;
}

export function skillScopeKey(scope: SkillScope): string {
  return scope.kind === "crew" ? `crew:${scope.groupId}` : scope.kind;
}

export interface Connector {
  id: ConnectorId;
  groupId: GroupId;
  service: string;
  account: string;
  envVar: string;
  note: string;
  secretSet: boolean;
  agents: AgentId[];
  secretHint: string;
  createdAt: number;
  updatedAt: number;
}

export interface ConnectorDraft {
  groupId: GroupId;
  service: string;
  account: string;
  envVar: string;
  note: string;
  secret: string;
  agents?: AgentId[];
}

export type PluginId = string;

export const CATALOG = [
  "neon",
  "cloudflare",
  "linear",
  "stripe",
  "agentmail",
  "supermemory",
  "firecrawl",
  "google",
] as const;

export type CatalogKind = (typeof CATALOG)[number];

export type PluginKind = CatalogKind | (string & {});

export interface PluginOffer {
  kind: CatalogKind;
  name: string;
  blurb: string;
  docs: string;
  endpoint: string;
  accountBacked: boolean;
}

export type PluginAccess = { mode: "everyone" } | { mode: "chosen"; agents: AgentId[] };

export interface PluginToolCard {
  name: string;
  description: string;
  access: PluginAccess;
}

export interface Plugin {
  id: PluginId;
  groupId: GroupId;
  kind: PluginKind;
  name: string;
  endpoint: string;
  custom: boolean;
  account: string;
  tools: PluginToolCard[];
  access: PluginAccess;
  connection: string;
  headers: string[];
  signedIn: boolean;
  connectedAt: number;
}

export interface HeaderPair {
  name: string;
  value: string;
}

export interface ServerReport {
  endpoint: string;
  transport: string;
  protocol: string;
  handshake: boolean;
  signin: SigninNeed;
  server: string;
  tools: string[];
  ms: number;
}

export type SigninNeed = "none" | "wanted" | "accepted" | "refused";

export type Surface = "computer" | "browser";

export interface Signin {
  agentId: AgentId;
  surface: Surface;
  domain: string;
  service: string;
  recognized: boolean;
  firstSeenAt: number;
  lastSeenAt: number;
}

export interface Computer {
  sandboxId: string;
  state: string;
  vncUrl: string | null;
}

export type Browser = { sessionId: string; state: string } & (
  | { liveViewUrl: string; unwatchable: null }
  | { liveViewUrl: null; unwatchable: string }
  | { liveViewUrl: null; unwatchable: null }
);

export interface Output {
  stdout: string;
  stderr: string;
  exitCode: number;
}

export interface AgentCard {
  id: AgentId;
  groupId: GroupId;
  sandboxId: string | null;
  browserId: string | null;
  hasComputer: boolean;
  hasBrowser: boolean;
  browserConsent: BrowserConsent;
  runsErrands: boolean;
  name: string;
  avatar: string;
  color: string;
  model: string;
  subscriptionModel: string;
  reasoningEffort?: ReasoningEffort | null;
  systemPrompt: string;
  skills: string[];
  lifecycle: Lifecycle;
  pinned: boolean;
  railOrder: number;
  version: number;
  createdAt: number;
  updatedAt: number;
  discardedAt: number | null;
}

export interface AgentDraft {
  groupId?: GroupId;
  name: string;
  avatar: string;
  color: string;
  model: string;
  subscriptionModel?: string;
  reasoningEffort?: ReasoningEffort | null;
  systemPrompt: string;
  skills: string[];
}

export type Activity =
  | { state: "idle" }
  | { state: "thinking" }
  | { state: "queued"; depth: number }
  | { state: "awaitingApproval" }
  | { state: "paused" };

export interface GuardLimits {
  maxHops: number;
  maxStepsPerRun: number;
  maxFanoutPerCall: number;
  maxSendsPerPair: number;
  maxToolRounds: number;
}

export type Provider = "compatible" | "chatgpt" | "claude";

export interface RankedModel {
  id: string;
  name: string;
  contextLength: number;
  promptPerMillion: number | null;
  completionPerMillion: number | null;
}

export interface ClaudeProgram {
  installed: boolean;
  version: string;
  install: string;
}

export interface Pickup {
  program: "claudeCode" | "codex";
  account: string;
  plan: string;
}

export interface Capabilities {
  loopbackEndpoints: boolean;
  claudeProvider: boolean;
  localFiles: boolean;
  borrowedSignins: boolean;
}

export interface Settings {
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
  defaultModel: string;
  subscriptionModel: string;
  reasoningEffort: ReasoningEffort;
  apiKeySet: boolean;
  apiKeyHint: string;
  requestTimeoutSecs: number;
  limits: GuardLimits;
  subscriptionModels: string[];
  quickActions?: QuickAction[];
  widgets?: Widget[];
  attention?: Attention;
  delivery?: DeliverySettings;
}

export type PushFormat = "ntfy" | "json";

export type PushContent = "outcome" | "generic";

export interface DeliverySettings {
  urlSet: boolean;
  urlHint: string;
  tokenSet: boolean;
  tokenHint: string;
  format: PushFormat;
  content: PushContent;
  linkBase: string;
}

export interface DeliveryPatch {
  url?: string;
  token?: string;
  format?: PushFormat;
  content?: PushContent;
  linkBase?: string;
}

export interface PushReport {
  lastSentAt: number | null;
  lastAttemptAt: number | null;
  lastError: string | null;
  retryAt: number | null;
}

export interface Attention {
  needsYou: boolean;
  failures: boolean;
  finished: boolean;
}

export type WidgetWidth = "narrow" | "wide";

export interface Widget {
  artifactId: ArtifactId;
  width: WidgetWidth;
  everyMinutes: number;
  addedBy: string;
}

export type QuickPlace =
  | { kind: "channel"; agentId: AgentId }
  | { kind: "calendar" }
  | { kind: "forYou" }
  | { kind: "settings"; section?: string | null }
  | { kind: "crewSettings"; groupId: GroupId };

export type QuickDoes =
  | { kind: "message"; agentId: AgentId; text: string }
  | { kind: "open"; place: QuickPlace };

export interface QuickAction {
  id: string;
  label: string;
  does: QuickDoes;
  addedBy: string;
}

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
  limits?: Partial<GuardLimits>;
  attention?: Partial<Attention>;
  delivery?: DeliveryPatch;
}

export interface ClaudeSigninStatus {
  signedIn: boolean;
}

export interface ClaudeSignin {
  id: string;
  url: string;
}

export interface SubscriptionStatus {
  signedIn: boolean;
  email: string;
  plan: string;
  includesCodex: boolean;
}

export interface AccountStatus {
  signedIn: boolean;
  email: string;
  origin: string;
  broker?: boolean;
}

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

export interface AccountConnection {
  id: string;
  provider: string;
  label: string;
  capabilities: string[];
}

export interface AccountConnectors {
  email: string;
  providers: AccountProvider[];
  connections: AccountConnection[];
}

export interface DeviceCode {
  verificationUrl: string;
  userCode: string;
  deviceAuthId: string;
  intervalSecs: number;
}

export type Reveal = { kind: "forYou" } | { kind: "agent"; id: AgentId };

export type UiEvent =
  | {
      type: "liveSnapshot";
      activity: Record<AgentId, Activity>;
      streams: Record<
        MessageId,
        {
          channelId: AgentId;
          agentId: AgentId;
          runId: RunId;
          to: Participant;
          text: string;
          truncated?: boolean;
        }
      >;
    }
  | { type: "agentsChanged" }
  | { type: "openUrl"; url: string; client?: string | null }
  | { type: "messageAppended"; message: Envelope }
  | {
      type: "streamStarted";
      messageId: MessageId;
      channelId: AgentId;
      agentId: AgentId;
      runId: RunId;
      to: Participant;
    }
  | { type: "streamDelta"; messageId: MessageId; channelId: AgentId; text: string }
  | { type: "reasoningDelta"; messageId: MessageId; text: string }
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
      cached: number;
      cost: number | null;
    }
  | { type: "runSettled"; runId: RunId; stepsUsed: number }
  | { type: "outcomeRecorded"; outcome: Outcome }
  | { type: "outcomesSeen"; runIds: RunId[] }
  | ({ type: "delivery" } & Delivery)
  | { type: "approvalRequested"; approvalId: ApprovalId; agentId: AgentId }
  | { type: "approvalSettled"; approvalId: ApprovalId; state: ApprovalState }
  | { type: "escalationRaised"; escalationId: EscalationId; agentId: AgentId }
  | { type: "decisionsChanged" }
  | { type: "decisionReminder"; count: number }
  | { type: "escalationCleared"; escalationId: EscalationId }
  | { type: "routinesChanged"; agentId: AgentId }
  | { type: "memoryChanged"; agentId: AgentId }
  | { type: "workingNotesChanged"; agentId: AgentId }
  | { type: "settingsChanged"; settings: Settings }
  | { type: "notebookChanged"; agentId: AgentId }
  | { type: "skillsChanged"; scope: SkillScope }
  | { type: "calendarChanged"; groupId: GroupId }
  | { type: "artifactsChanged"; groupId: GroupId };

export interface Tokens {
  prompt: number;
  completion: number;
  cost: number | null;
  calls: number;
}

export interface GroupUsage extends Tokens {
  groupId: GroupId;
}

export interface RunUsage extends Tokens {
  runId: RunId;
}

export interface GroupReset {
  messages: number;
  routines: number;
  notes: number;
  workingNotes: number;
  calls: number;
}

export type OccasionId = string;

export interface Occasion {
  id: OccasionId;
  groupId: GroupId;
  agentId: AgentId | null;
  title: string;
  detail: string;
  place: string;
  startsAt: number;
  minutes: number | null;
  allDay: boolean;
  createdAt: number;
  updatedAt: number;
}

export interface OccasionDraft {
  groupId: GroupId;
  title: string;
  detail: string;
  place: string;
  startsAt: string;
  minutes: number | null;
}

export type ArtifactId = string;

export type ArtifactActor = { kind: "operator" } | { kind: "agent"; id: AgentId; name: string };

export interface Artifact {
  id: ArtifactId;
  groupId: GroupId;
  owner: { id: AgentId; name: string; gone: boolean } | null;
  title: string;
  version: number;
  editedBy: ArtifactActor;
  createdAt: number;
  updatedAt: number;
  sources: ArtifactSource[];
  sourcesAllowed: boolean;
  condensed: boolean;
}

export interface CondensedView {
  artifact: Artifact;
  page: string | null;
}

export interface ArtifactSource {
  name: string;
  tool: string;
  arguments: unknown;
}

export interface ArtifactRead {
  name: string;
  data: unknown;
  text: string | null;
  error: string | null;
}

export type ArtifactChange = "created" | "edited" | "restored" | "took" | "handed" | "allowed";

export interface ArtifactEntry {
  seq: number;
  at: number;
  change: ArtifactChange;
  by: ArtifactActor;
  version: number;
  owner: string | null;
  note: string;
}

export interface ArtifactDetail {
  artifact: Artifact;
  log: ArtifactEntry[];
}

export interface ArtifactMade {
  id: ArtifactId;
  version: number;
  title: string;
}

export type RoutineId = string;

export type TriggerSpec = string;

export interface Routine {
  id: RoutineId;
  agentId: AgentId;
  name: string;
  what: string;
  trigger: TriggerSpec;
  active: boolean;
  skipIfWorking: boolean;
  nextRunAt: number | null;
  lastRunAt: number | null;
  createdAt: number;
}

export interface UpcomingRun {
  routineId: RoutineId;
  agentId: AgentId;
  groupId: GroupId;
  name: string;
  nextRunAt: number;
}

export type RunKind = "scheduled" | "test" | "skipped" | "event" | "failed";

export interface RoutineRun {
  runId: RunId | null;
  kind: RunKind;
  at: number;
  error?: string | null;
  spent: Tokens;
  missed: number;
}

export interface WebhookAddress {
  url?: string | null;
}

export interface RoutineDraft {
  name: string;
  what: string;
  trigger: TriggerSpec;
  inSecs: number | null;
  skipIfWorking: boolean;
}

export interface SearchHits {
  messages: MessageHit[];
  files: FileHit[];
  links: LinkHit[];
  routines: Routine[];
  artifacts: Artifact[];
}

export interface MessageHit {
  id: MessageId;
  channelId: AgentId;
  from: Participant;
  to: Participant;
  excerpt: string;
  createdAt: number;
}

export interface FileHit {
  file: Attachment;
  messageId: MessageId;
  channelId: AgentId;
  from: Participant;
  createdAt: number;
}

export interface LinkHit {
  url: string;
  messageId: MessageId;
  channelId: AgentId;
  createdAt: number;
}

export interface Presence {
  roster: Record<AgentId, { name: string; crew: GroupId }>;
  crews: { id: GroupId; name: string }[];
  activity: Record<AgentId, Activity>;
  waiting: Approval[];
  stuck: Escalation[];
  decisions: WorkDecision[];
  session: Tokens;
  running: number;
  finished: number;
}

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
  detail?: string;
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

export function errorMessage(value: unknown): string {
  if (isCommandError(value)) return value.message;
  if (value instanceof Error) return value.message;
  if (typeof value === "string") return value;
  return "Something went wrong.";
}

export function asSentence(text: string): string {
  const trimmed = text.trim();
  if (trimmed === "") return trimmed;
  const opened = trimmed.replace(/^[a-z]/, (letter) => letter.toUpperCase());
  return /[.!?…:]$/.test(opened) ? opened : `${opened}.`;
}

export const TIDY_LABEL = "Overnight memory tidy";

export function plainText(envelope: Envelope): string {
  return envelope.parts
    .map((part) => {
      if (part.type === "text") return part.text;
      if (part.type === "routine") return part.what;
      if (part.type === "tidy") return TIDY_LABEL;
      if (part.type === "decision") return part.answer;
      return null;
    })
    .filter((text): text is string => text !== null)
    .join("\n")
    .trim();
}

export function isInterAgent(envelope: Envelope): boolean {
  return envelope.from.kind === "agent" && envelope.to.kind === "agent";
}

export interface WorkingNote {
  at: number;
  body: string;
}

export interface DecisionRequest {
  question: string;
  context: string;
  recommendation: string;
  options: string[];
  source: string;
}

export interface WorkDecision {
  id: string;
  agentId: AgentId;
  groupId: GroupId;
  topic: string;
  request: DecisionRequest;
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

export type OutcomeTrigger = "operator" | "routine" | "event" | "decision" | "retry";

export type OutcomeStatus =
  | "done"
  | "nothingNew"
  | "needsYou"
  | "failed"
  | "stopped"
  | "interrupted";

export type OutcomeNeed =
  | { kind: "decision"; id: string }
  | { kind: "escalation"; id: EscalationId };

export interface Outcome {
  runId: RunId;
  agentId: AgentId;
  groupId: GroupId;
  trigger: OutcomeTrigger;
  triggerRef: string | null;
  status: OutcomeStatus;
  headline: string;
  noteId: MessageId | null;
  needs: OutcomeNeed[];
  steps: number;
  startedAt: number;
  finishedAt: number;
  seenAt: number | null;
}

export type DeliveryId = string;

export type DeliveryKind = "outcome" | "needsYou" | "reminder" | "failure";

export type DeliveryTarget = { kind: "channel"; agentId: AgentId } | { kind: "forYou" };

export interface Delivery {
  id: DeliveryId;
  kind: DeliveryKind;
  title: string;
  body: string;
  target: DeliveryTarget;
  createdAt: number;
}

export type NotificationAccess = "allowed" | "notAsked" | "denied" | "unavailable";
