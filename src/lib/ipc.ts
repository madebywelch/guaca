import type { GroupArchive, Reconnect } from "./transfer";

import {
  attached,
  desktop,
  hosted,
  invoke,
  invokeLocal,
  notify,
  PAGE_ID,
  type Refusal,
  type Remote,
  openExternal as reachBrowser,
  subscribe,
  token,
  type Unlisten,
  upload,
  workspaceOrigin,
} from "./transport";

import type {
  AccountConnectors,
  AccountStatus,
  Activity,
  AgentCard,
  AgentDraft,
  AgentId,
  Approval,
  ApprovalId,
  ApprovalState,
  Artifact,
  ArtifactDetail,
  ArtifactId,
  ArtifactRead,
  ArtifactSource,
  Attachment,
  Browser,
  BrowserConsent,
  Capabilities,
  CatalogKind,
  ClaudeProgram,
  Computer,
  CondensedView,
  Connector,
  ConnectorDraft,
  ConnectorId,
  Decision,
  Delivery,
  DeliveryId,
  DeviceCode,
  DirectoryBoard,
  DirectoryListing,
  DirectoryPage,
  DirectorySkill,
  Envelope,
  Escalation,
  EscalationId,
  Group,
  GroupDraft,
  GroupId,
  GroupReset,
  GroupUsage,
  HeaderPair,
  MessageId,
  NotebookEntry,
  NotificationAccess,
  Occasion,
  OccasionDraft,
  OccasionId,
  OperatorView,
  Outcome,
  PermissionRule,
  PermissionRuleDraft,
  Pickup,
  Plugin,
  PluginAccess,
  PluginId,
  PluginOffer,
  Presence,
  ProtectedAction,
  PushReport,
  QuickDoes,
  RankedModel,
  Reveal,
  Routine,
  RoutineDraft,
  RoutineId,
  RoutineRun,
  RunId,
  Running,
  RunUsage,
  SearchHits,
  ServerReport,
  Settings,
  SettingsPatch,
  Signin,
  Skill,
  SkillDraft,
  SkillScope,
  Staged,
  SubscriptionStatus,
  TaskRecord,
  ToolSummary,
  UiEvent,
  UpcomingRun,
  WebhookAddress,
  WidgetWidth,
  WorkDecision,
  WorkingNote,
} from "./types";
import { errorMessage } from "./types";

const EVENT_CHANNEL = "guac://event";

const REVEAL_CHANNEL = "guac://reveal";

type PanelAssignment = Remote & { generation: number };

export const api = {
  listPermissionRules: (groupId: GroupId | null) =>
    invoke<PermissionRule[]>("list_permission_rules", { groupId }),
  savePermissionRule: (rule: PermissionRuleDraft, id: string | null, version: number | null) =>
    invoke<PermissionRule>("save_permission_rule", { rule, id, version }),
  deletePermissionRule: (id: string, groupId: GroupId | null, version: number) =>
    invoke<void>("delete_permission_rule", { id, groupId, version }),
  exportGroup: (id: GroupId) => invoke<GroupArchive>("export_group", { id }),
  importGroup: (archive: GroupArchive, name: string) =>
    invoke<Group>("import_group", { archive, name }),
  groupReconnect: (id: GroupId) => invoke<Reconnect[]>("group_reconnect", { id }),
  agentComputer: (id: AgentId) => invoke<Computer | null>("agent_computer", { id }),

  giveAgentComputer: (id: AgentId) => invoke<void>("give_agent_computer", { id }),

  takeAgentComputer: (id: AgentId) => invoke<void>("take_agent_computer", { id }),

  startAgentComputer: (id: AgentId) => invoke<Computer>("start_agent_computer", { id }),

  stopAgentComputer: (id: AgentId) => invoke<Computer | null>("stop_agent_computer", { id }),

  deleteAgentComputer: (id: AgentId) => invoke<void>("delete_agent_computer", { id }),

  agentBrowser: (id: AgentId) => invoke<Browser | null>("agent_browser", { id }),

  giveAgentBrowser: (id: AgentId) => invoke<void>("give_agent_browser", { id }),

  takeAgentBrowser: (id: AgentId) => invoke<void>("take_agent_browser", { id }),

  setAgentBrowserConsent: (id: AgentId, consent: BrowserConsent) =>
    invoke<void>("set_agent_browser_consent", { id, consent }),

  setAgentErrands: (id: AgentId, given: boolean) =>
    invoke<void>("set_agent_errands", { id, given }),

  setAgentWeb: (id: AgentId, given: boolean) => invoke<void>("set_agent_web", { id, given }),

  startAgentBrowser: (id: AgentId) => invoke<Browser>("start_agent_browser", { id }),

  stopAgentBrowser: (id: AgentId) => invoke<void>("stop_agent_browser", { id }),

  groupConnectors: (groupId: GroupId) => invoke<Connector[]>("group_connectors", { groupId }),

  createConnector: (draft: ConnectorDraft) => invoke<Connector>("create_connector", { draft }),

  updateConnector: (id: ConnectorId, agents: AgentId[], secret: string | null) =>
    invoke<void>("update_connector", { id, agents, secret }),
  deleteConnector: (id: ConnectorId) => invoke<void>("delete_connector", { id }),

  addQuickAction: (label: string, does: QuickDoes) =>
    invoke<Settings>("add_quick_action", { label, does }),
  removeQuickAction: (id: string) => invoke<Settings>("remove_quick_action", { id }),

  builtinTools: () => invoke<ToolSummary[]>("builtin_tools"),

  reportView: (view: OperatorView, presence?: { focused: boolean; client: string }) =>
    invoke<void>("report_view", {
      view,
      focused: presence?.focused ?? null,
      client: presence?.client ?? null,
    }),

  listSkills: (scope: SkillScope) => invoke<Skill[]>("list_skills", { scope }),
  readSkill: (scope: SkillScope, name: string) => invoke<Skill>("read_skill", { scope, name }),
  saveSkill: (scope: SkillScope, draft: SkillDraft) =>
    invoke<Skill>("save_skill", { scope, draft }),
  deleteSkill: (scope: SkillScope, name: string) =>
    invoke<boolean>("delete_skill", { scope, name }),
  skillDirectory: (board: DirectoryBoard, page: number) =>
    invoke<DirectoryPage>("skill_directory", { board, page }),
  searchSkillDirectory: (query: string) =>
    invoke<DirectoryListing[]>("search_skill_directory", { query }),
  previewDirectorySkill: (id: string) => invoke<DirectorySkill>("preview_directory_skill", { id }),
  addDirectorySkill: (scope: SkillScope, id: string, hash: string) =>
    invoke<Skill>("add_directory_skill", { scope, id, hash }),

  claudeProgram: () => invoke<ClaudeProgram>("claude_program"),

  existingSignins: () => invoke<Pickup[]>("existing_signins"),

  pluginCatalog: () => invoke<PluginOffer[]>("plugin_catalog"),

  groupPlugins: (groupId: GroupId) => invoke<Plugin[]>("group_plugins", { groupId }),

  connectPlugin: (groupId: GroupId, kind: CatalogKind, connection?: string) =>
    invoke<Plugin>("connect_plugin", { groupId, kind, connection: connection ?? null }),

  addPlugin: (groupId: GroupId, name: string, url: string, key?: string, headers?: HeaderPair[]) =>
    invoke<Plugin>("add_plugin", { groupId, name, url, key: key ?? null, headers }),

  readdressPlugin: (
    groupId: GroupId,
    id: PluginId,
    url: string,
    key?: string,
    headers?: HeaderPair[],
  ) => invoke<Plugin>("readdress_plugin", { groupId, id, url, key: key ?? null, headers }),

  probeServer: (url: string, key?: string, headers?: HeaderPair[]) =>
    invoke<ServerReport>("probe_server", { url, key: key ?? null, headers }),

  checkPlugin: (id: PluginId) => invoke<ServerReport>("check_plugin", { id }),

  setPluginAccess: (id: PluginId, access: PluginAccess) =>
    invoke<Plugin>("set_plugin_access", { id, access }),

  setPluginTool: (id: PluginId, tool: string, access: PluginAccess) =>
    invoke<Plugin>("set_plugin_tool", { id, tool, access }),

  disconnectPlugin: (id: PluginId) => invoke<void>("disconnect_plugin", { id }),

  agentSignins: (id: AgentId) => invoke<Signin[]>("agent_signins", { id }),

  scanAgentSignins: (id: AgentId) => invoke<Signin[]>("scan_agent_signins", { id }),

  approvalStates: () => invoke<Record<ApprovalId, ApprovalState>>("approval_states"),
  approvalAnswers: () => invoke<Record<ApprovalId, string>>("approval_answers"),
  listDecisions: () => invoke<WorkDecision[]>("list_decisions"),
  listOutcomes: (query: {
    since?: number;
    groupId?: GroupId;
    unseenOnly?: boolean;
    limit: number;
  }) =>
    invoke<Outcome[]>("list_outcomes", {
      since: query.since ?? null,
      groupId: query.groupId ?? null,
      unseenOnly: query.unseenOnly ?? null,
      limit: query.limit,
    }),
  runningRuns: () => invoke<Running[]>("running_runs"),
  markOutcomesSeen: (target: { runIds?: RunId[]; before?: number }) =>
    invoke<RunId[]>("mark_outcomes_seen", {
      runIds: target.runIds ?? null,
      before: target.before ?? null,
    }),
  pendingDeliveries: () => invoke<Delivery[]>("pending_deliveries", { client: PAGE_ID }),
  claimDelivery: (id: DeliveryId) => invoke<boolean>("claim_delivery", { id, client: PAGE_ID }),
  releaseDelivery: (id: DeliveryId) => invoke<boolean>("release_delivery", { id, client: PAGE_ID }),
  pushReport: () => invoke<PushReport>("push_report"),
  testPush: () => invoke<string>("test_push"),
  answerDecision: (id: string, answer: string, updatedAt: number) =>
    invoke<WorkDecision>("answer_decision", { id, answer, updatedAt }),
  resumeDecision: (id: string) => invoke<WorkDecision>("resume_decision", { id }),
  snoozeDecision: (id: string, until: number) => invoke<void>("snooze_decision", { id, until }),
  pendingApprovals: async () =>
    (await invoke<Approval[]>("pending_approvals")).map((one) => ({
      ...one,
      lapsesAt: one.lapsesAt ?? null,
    })),

  openEscalations: () => invoke<Escalation[]>("open_escalations"),
  clearEscalation: (id: EscalationId) => invoke<void>("clear_escalation", { id }),

  calendar: (from: number, until: number, groupId?: GroupId) =>
    invoke<Occasion[]>("calendar", { from, until, groupId: groupId ?? null }),
  createOccasion: (draft: OccasionDraft) => invoke<Occasion>("create_occasion", { draft }),
  updateOccasion: (id: OccasionId, draft: OccasionDraft) =>
    invoke<Occasion>("update_occasion", { id, draft }),
  deleteOccasion: (id: OccasionId) => invoke<void>("delete_occasion", { id }),

  artifacts: (groupId?: GroupId | null) =>
    invoke<Artifact[]>("artifacts", { groupId: groupId ?? null }),
  artifactDetail: (id: ArtifactId) => invoke<ArtifactDetail>("artifact_detail", { id }),
  artifactPage: (id: ArtifactId, version?: number | null) =>
    invoke<string>("artifact_page", { id, version: version ?? null }),
  artifactData: (id: ArtifactId, version?: number | null) =>
    invoke<ArtifactRead[]>("artifact_data", { id, version: version ?? null }),
  allowArtifactSources: (
    id: ArtifactId,
    version: number,
    ownerId: AgentId | null,
    sources: ArtifactSource[],
  ) => invoke<Artifact>("approve_artifact_sources", { id, version, ownerId, sources }),
  restoreArtifact: (id: ArtifactId, version: number) =>
    invoke<Artifact>("restore_artifact", { id, version }),
  handArtifact: (id: ArtifactId, agentId: AgentId) =>
    invoke<Artifact>("hand_artifact", { id, agentId }),
  deleteArtifact: (id: ArtifactId) => invoke<void>("delete_artifact", { id }),
  artifactCondensed: (id: ArtifactId) => invoke<CondensedView>("artifact_condensed", { id }),
  pinArtifact: (id: ArtifactId, width: WidgetWidth) =>
    invoke<Settings>("pin_artifact", { id, width }),
  unpinArtifact: (id: ArtifactId) => invoke<Settings>("unpin_artifact", { id }),

  decideApproval: (id: ApprovalId, decision: Decision) =>
    invoke<Approval>("decide_approval", { id, decision }),
  answerQuestion: (id: ApprovalId, answer: string) =>
    invoke<Approval>("answer_question", { id, answer }),

  agentGrants: (id: AgentId) => invoke<ProtectedAction[]>("agent_grants", { id }),

  revokeGrant: (id: AgentId, action: ProtectedAction) =>
    invoke<ProtectedAction[]>("revoke_grant", { id, action }),

  listGroups: () => invoke<Group[]>("list_groups"),

  createGroup: (draft: GroupDraft) => invoke<Group>("create_group", { draft }),

  updateGroup: (id: GroupId, draft: GroupDraft) => invoke<Group>("update_group", { id, draft }),

  testGroupConnection: (id: GroupId | null, draft: GroupDraft) =>
    invoke<string>("test_group_connection", { id, draft }),

  deleteGroup: (id: GroupId) => invoke<void>("delete_group", { id }),

  disbandGroup: (id: GroupId) => invoke<void>("disband_group", { id }),

  listAgents: () => invoke<AgentCard[]>("list_agents"),

  createAgent: (draft: AgentDraft) => invoke<AgentCard>("create_agent", { draft }),

  updateAgent: (id: AgentId, draft: AgentDraft) => invoke<AgentCard>("update_agent", { id, draft }),

  deleteAgent: (id: AgentId) => invoke<void>("delete_agent", { id }),

  restoreAgent: (id: AgentId) => invoke<AgentCard>("restore_agent", { id }),

  purgeAgent: (id: AgentId) => invoke<void>("purge_agent", { id }),

  duplicateAgent: (id: AgentId) => invoke<AgentCard>("duplicate_agent", { id }),

  hireAgents: (groupId: GroupId, drafts: AgentDraft[], idempotencyKey?: string) =>
    invoke<AgentCard[]>("hire_agents", { groupId, drafts, idempotencyKey }),

  setAgentPaused: (id: AgentId, paused: boolean) =>
    invoke<AgentCard>("set_agent_paused", { id, paused }),

  setAgentPinned: (id: AgentId, pinned: boolean) =>
    invoke<AgentCard>("set_agent_pinned", { id, pinned }),

  moveAgent: (id: AgentId, groupId: GroupId, before: AgentId | null) =>
    invoke<AgentCard>("move_agent", { id, groupId, before }),

  moveGroup: (id: GroupId, before: GroupId | null) => invoke<Group[]>("move_group", { id, before }),

  agentActivity: () => invoke<Record<AgentId, Activity>>("agent_activity"),

  agentLastActive: () => invoke<Record<AgentId, number>>("agent_last_active"),

  agentMemory: (id: AgentId) => invoke<string>("agent_memory", { id }),
  agentMemoryVersioned: (id: AgentId) =>
    invoke<{ content: string; version: string }>("agent_memory_versioned", { id }),

  setAgentMemory: (id: AgentId, content: string, expected?: string) =>
    invoke<string>("set_agent_memory", { id, content, expected }),

  agentNotebook: (id: AgentId) => invoke<NotebookEntry[]>("agent_notebook", { id }),
  readNotebook: (id: AgentId, path: string) => invoke<string>("read_notebook", { id, path }),
  deleteNotebookFile: (id: AgentId, path: string) =>
    invoke<boolean>("delete_notebook_file", { id, path }),
  agentWorkingNotes: (id: AgentId) => invoke<WorkingNote[]>("agent_working_notes", { id }),

  clearAgentWorkingNotes: (id: AgentId) => invoke<void>("clear_agent_working_notes", { id }),

  channelMessages: (channelId: AgentId, limit?: number, through?: MessageId) =>
    invoke<Envelope[]>("channel_messages", { channelId, limit, through }),

  taskRecord: (channelId: AgentId, runId: RunId) =>
    invoke<TaskRecord | null>("task_record", { channelId, runId }),

  pairMessages: (a: AgentId, b: AgentId, limit?: number) =>
    invoke<Envelope[]>("pair_messages", { a, b, limit }),

  conversationFlow: (group: GroupId, limit?: number) =>
    invoke<Envelope[]>("conversation_flow", { group, limit }),

  search: (query: string, limit?: number) => invoke<SearchHits>("search", { query, limit }),

  forwardFiles: (origin: string, token: string, paths: string[]) =>
    invokeLocal<Staged>("forward_files", { origin, token, paths }),

  reportPresence: (presence: Presence | null) => invokeLocal<void>("report_presence", { presence }),

  reportHost: (host: Remote, generation?: number) =>
    invokeLocal<boolean>("report_host", { host, generation }),

  notificationAccess: () => invokeLocal<NotificationAccess | null>("notification_access"),

  openNotificationSettings: () => invokeLocal<void>("open_notification_settings"),

  menubarHost: () => invokeLocal<PanelAssignment | null>("menubar_host"),

  openWindow: (target: Reveal | null) => invokeLocal<void>("open_window", { target }),

  closeMenubar: () => invokeLocal<void>("close_menubar"),

  fitMenubar: (height: number) => invokeLocal<void>("fit_menubar", { height }),

  stopEverything: () => invoke<number>("stop_everything"),

  stageUploads: async (files: File[]): Promise<Staged> => {
    const staged: Staged = { attached: [], refused: [] };
    for (const file of files) {
      try {
        staged.attached.push(await upload<Attachment>(file));
      } catch (error) {
        staged.refused.push(errorMessage(error));
      }
    }
    return staged;
  },

  saveFile: (digest: string, name: string) =>
    invokeLocal<string>("download_file", {
      origin: workspaceOrigin(),
      token: token(),
      digest,
      name,
    }),

  frameArtifact: (html: string) =>
    invoke<{ port: number; id: string; ticket: string | null }>("frame_artifact", { html }),

  sendMessage: (
    agentId: AgentId,
    text: string,
    files: Attachment[] = [],
    idempotencyKey?: string,
  ) =>
    invoke<RunId | { messageId: string; runId: RunId }>("send_message", {
      agentId,
      text,
      files: files.map(({ digest, name }) => ({ digest, name })),
      idempotencyKey,
    }),

  clearChannel: (channelId: AgentId) => invoke<number>("clear_channel", { channelId }),

  retryTurn: (agentId: AgentId, messageId: MessageId) =>
    invoke<RunId>("retry_turn", { agentId, messageId }),

  stopRun: (runId: RunId) => invoke<boolean>("stop_run", { runId }),
  clearGroup: (groupId: GroupId) => invoke<GroupReset>("clear_group", { groupId }),
  agentRoutines: (id: AgentId) => invoke<Routine[]>("agent_routines", { id }),
  upcomingRuns: (until: number) => invoke<UpcomingRun[]>("upcoming_runs", { until }),
  createRoutine: (agentId: AgentId, draft: RoutineDraft) =>
    invoke<Routine>("create_routine", { agentId, draft }),
  updateRoutine: (id: RoutineId, draft: RoutineDraft) =>
    invoke<Routine>("update_routine", { id, draft }),

  setRoutineActive: (id: RoutineId, active: boolean) =>
    invoke<Routine>("set_routine_active", { id, active }),

  testRoutine: (id: RoutineId) => invoke<RunId>("test_routine", { id }),

  routineRuns: (id: RoutineId) => invoke<RoutineRun[]>("routine_runs", { id }),

  webhookAddress: () => invoke<WebhookAddress>("webhook_address"),
  revealWebhookSecret: () => invoke<string>("reveal_webhook_secret"),

  deleteRoutine: (id: RoutineId) => invoke<void>("delete_routine", { id }),
  usageSummary: () => invoke<GroupUsage[]>("usage_summary"),
  usageForRuns: (runs: RunId[]) => invoke<RunUsage[]>("usage_for_runs", { runs }),

  capabilities: () => invoke<Capabilities>("capabilities"),

  getSettings: () => invoke<Settings>("get_settings"),

  updateSettings: (patch: SettingsPatch) => invoke<Settings>("update_settings", { patch }),

  testConnection: (patch?: SettingsPatch) => invoke<string>("test_connection", { patch }),

  rankedModels: (category: string) => invoke<RankedModel[]>("ranked_models", { category }),

  subscriptionModels: () => invoke<import("./types").SubscriptionModel[]>("subscription_models"),

  subscriptionStatus: () => invoke<SubscriptionStatus>("subscription_status"),

  claudeSigninStatus: () => invoke<import("./types").ClaudeSigninStatus>("claude_signin_status"),
  beginClaudeSignin: () => invoke<import("./types").ClaudeSignin>("begin_claude_signin"),
  completeClaudeSignin: (id: string, code: string) =>
    invoke<import("./types").ClaudeSigninStatus>("complete_claude_signin", { id, code }),
  cancelClaudeSignin: (id: string) => invoke<void>("cancel_claude_signin", { id }),

  beginSubscriptionSignin: () => invoke<DeviceCode>("begin_subscription_signin"),

  completeSubscriptionSignin: (code: DeviceCode) =>
    invoke<SubscriptionStatus>("complete_subscription_signin", { code }),

  signOutSubscription: () => invoke<Settings>("sign_out_subscription"),

  accountStatus: () => invoke<AccountStatus>("account_status"),

  signInAccount: () => invoke<AccountStatus>("sign_in_account"),

  accountConnectors: () => invoke<AccountConnectors>("account_connectors"),

  signOutAccount: () => invoke<AccountStatus>("sign_out_account"),

  setPluginConnection: (groupId: GroupId, kind: CatalogKind, connection: string) =>
    invoke<Plugin>("set_plugin_connection", { groupId, kind, connection }),
};

export function openExternal(url: string): Promise<void> {
  return reachBrowser(url);
}

export async function notifyOperator(title: string, body: string): Promise<boolean> {
  return notify(title, body);
}

export function onRuntimeEvent(
  handler: (event: UiEvent) => void,
  onReconnect?: () => void,
  onLost?: (error: Refusal) => void,
  onClose?: () => void,
): Promise<Unlisten> {
  return subscribe(EVENT_CHANNEL, handler, onReconnect, onLost, onClose);
}

export function onRevealRequest(handler: (target: Reveal) => void): Promise<Unlisten> {
  if (hosted && !attached()) return Promise.resolve(() => {});
  return import("@tauri-apps/api/event").then((events) =>
    events.listen<Reveal>(REVEAL_CHANNEL, (message) => handler(message.payload)),
  );
}

export async function onFileDrop(handlers: {
  dropped: (staged: Promise<Staged>) => void;
  over: (inside: boolean) => void;
}): Promise<Unlisten> {
  if (!desktop) {
    let depth = 0;
    const enter = (event: DragEvent) => {
      if (!event.dataTransfer?.types.includes("Files")) return;
      event.preventDefault();
      depth += 1;
      handlers.over(true);
    };
    const over = (event: DragEvent) => {
      if (!event.dataTransfer?.types.includes("Files")) return;
      event.preventDefault();
    };
    const leave = (event: DragEvent) => {
      if (!event.dataTransfer?.types.includes("Files")) return;
      depth = Math.max(0, depth - 1);
      if (depth === 0) handlers.over(false);
    };
    const drop = (event: DragEvent) => {
      const files = Array.from(event.dataTransfer?.files ?? []);
      if (files.length === 0) return;
      event.preventDefault();
      depth = 0;
      handlers.over(false);
      handlers.dropped(api.stageUploads(files));
    };
    window.addEventListener("dragenter", enter);
    window.addEventListener("dragover", over);
    window.addEventListener("dragleave", leave);
    window.addEventListener("drop", drop);
    return () => {
      window.removeEventListener("dragenter", enter);
      window.removeEventListener("dragover", over);
      window.removeEventListener("dragleave", leave);
      window.removeEventListener("drop", drop);
    };
  }

  const events = await import("@tauri-apps/api/event");
  const stops = await Promise.all([
    events.listen(events.TauriEvent.DRAG_ENTER, () => handlers.over(true)),
    events.listen(events.TauriEvent.DRAG_LEAVE, () => handlers.over(false)),
    events.listen<{ paths: string[] }>(events.TauriEvent.DRAG_DROP, (message) => {
      handlers.over(false);
      const paths = message.payload.paths ?? [];
      handlers.dropped(api.forwardFiles(workspaceOrigin(), token(), paths));
    }),
  ]);
  return () => {
    for (const stop of stops) stop();
  };
}
