/**
 * Typed wrappers over the Tauri command surface.
 *
 * Every call the UI can make goes through here, so the set of things the
 * webview is able to do is one readable list. Tauri maps camelCase argument
 * keys onto the Rust snake_case parameters.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, TauriEvent, type UnlistenFn } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { openUrl } from "@tauri-apps/plugin-opener";

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
  Attachment,
  Browser,
  CatalogKind,
  Computer,
  Connector,
  ConnectorDraft,
  ConnectorId,
  Decision,
  DeviceCode,
  Envelope,
  Group,
  GroupDraft,
  GroupId,
  GroupReset,
  GroupUsage,
  Harness,
  HarnessOnMachine,
  HeaderPair,
  MessageId,
  Plugin,
  PluginAccess,
  PluginId,
  PluginOffer,
  ProtectedAction,
  RankedModel,
  RepoStatus,
  Repository,
  RepositoryDraft,
  RepositoryId,
  Routine,
  RoutineDraft,
  RoutineId,
  RoutineRun,
  RunId,
  RunUsage,
  SearchHits,
  ServerReport,
  Settings,
  SettingsPatch,
  Signin,
  Staged,
  SubscriptionStatus,
  UiEvent,
  WorkingNote,
} from "./types";

const EVENT_CHANNEL = "guac://event";

/**
 * The menu bar asking the window to open one agent's channel.
 *
 * Its own channel rather than a `UiEvent`. That one is the runtime saying what
 * happened, and this is one surface asking another to go somewhere: folding
 * them together would put a case in the transcript's event handling for
 * something the runtime never emits. Kept in step with `tray.rs`.
 */
const REVEAL_CHANNEL = "guac://reveal";

export const api = {
  /** `null` when the agent has never been given a computer. */
  agentComputer: (id: AgentId) => invoke<Computer | null>("agent_computer", { id }),

  /**
   * Lets this agent have a computer. The decision alone: no machine is made
   * here, and one is only rented when the agent or the operator needs it.
   */
  giveAgentComputer: (id: AgentId) => invoke<void>("give_agent_computer", { id }),

  /** Takes it back, and sleeps the machine if there is one. The disk is kept. */
  takeAgentComputer: (id: AgentId) => invoke<void>("take_agent_computer", { id }),

  /** Creates or wakes the sandbox, and brings the desktop up. Idempotent. */
  startAgentComputer: (id: AgentId) => invoke<Computer>("start_agent_computer", { id }),

  /** Puts it to sleep. The disk is kept, so a signed-in browser stays signed in. */
  stopAgentComputer: (id: AgentId) => invoke<Computer | null>("stop_agent_computer", { id }),

  /** Destroys the sandbox and everything on its disk. */
  deleteAgentComputer: (id: AgentId) => invoke<void>("delete_agent_computer", { id }),

  /** `null` when the agent has no browser, or the one it had has gone. */
  agentBrowser: (id: AgentId) => invoke<Browser | null>("agent_browser", { id }),

  /** Lets this agent have a browser. The decision alone; nothing is opened. */
  giveAgentBrowser: (id: AgentId) => invoke<void>("give_agent_browser", { id }),

  /**
   * Takes it back, and closes the browser if one is open. What it is signed in
   * to is saved to the agent's profile, so giving it back opens it signed in.
   */
  takeAgentBrowser: (id: AgentId) => invoke<void>("take_agent_browser", { id }),

  /** Opens one, or hands back the one it already has. Idempotent. */
  startAgentBrowser: (id: AgentId) => invoke<Browser>("start_agent_browser", { id }),

  /**
   * Ends it, keeping what it is signed in to.
   *
   * Closing is what writes the cookies back to the agent's profile, so this is
   * how a sign-in just performed is made durable rather than waiting for the
   * idle timeout to do it.
   */
  stopAgentBrowser: (id: AgentId) => invoke<void>("stop_agent_browser", { id }),

  /** Every account a crew can reach. Never carries a credential's value. */
  groupConnectors: (groupId: GroupId) => invoke<Connector[]>("group_connectors", { groupId }),

  createConnector: (draft: ConnectorDraft) => invoke<Connector>("create_connector", { draft }),

  deleteConnector: (id: ConnectorId) => invoke<void>("delete_connector", { id }),

  /**
   * The directories a crew has linked, and who in it may work in each.
   *
   * No disk is touched: a repository moved or deleted since it was linked still
   * comes back, because this panel is where the operator fixes that.
   */
  /**
   * Every repository in the workspace. One read for the whole rail, which draws
   * crews and their contents from one roster.
   */
  listRepositories: () => invoke<Repository[]>("list_repositories"),

  /**
   * What every linked repository is doing, by id. One call for the whole rail.
   *
   * A repository that could not be read is absent rather than present and
   * empty: the directory may have been moved since it was linked, and a row
   * saying "main, clean" about a path that is gone is worse than one saying
   * nothing.
   */
  repositoryStatuses: () => invoke<Record<RepositoryId, RepoStatus>>("repository_statuses"),

  groupRepositories: (groupId: GroupId) => invoke<Repository[]>("group_repositories", { groupId }),

  /**
   * Links a directory, after checking with git that it is the root of a work
   * tree. Nobody is given it here: that is `setRepositoryAccess`.
   */
  createRepository: (draft: RepositoryDraft) => invoke<Repository>("create_repository", { draft }),

  /**
   * Renames one, rewrites the line its agents read, or changes which program
   * does the writing. The path is not among them: a different directory is a
   * different repository, because reach was granted for that one.
   */
  updateRepository: (id: RepositoryId, name: string, note: string, harness: Harness) =>
    invoke<Repository>("update_repository", { id, name, note, harness }),

  /** Unlinks it. Nothing on disk is touched. */
  deleteRepository: (id: RepositoryId) => invoke<void>("delete_repository", { id }),

  /**
   * Which coding harnesses are on this machine, and how to get the ones that
   * are not. Everything else about a job is discovered when it runs; this
   * cannot be, because the refusal would reach an agent minutes later rather
   * than the person choosing.
   */
  codingHarnesses: () => invoke<HarnessOnMachine[]>("coding_harnesses"),

  /**
   * Gives one agent a repository, or takes it back. One agent per call, so a
   * panel that is a tick behind cannot revoke somebody while granting somebody
   * else.
   */
  /**
   * Puts one agent in a repository, or takes it out with `null`.
   *
   * A move rather than a grant: an agent works in at most one, so there is no
   * second call that takes one away and no list to send.
   */
  setAgentRepository: (id: AgentId, repositoryId: RepositoryId | null) =>
    invoke<AgentCard>("set_agent_repository", { id, repositoryId }),

  pluginCatalog: () => invoke<PluginOffer[]>("plugin_catalog"),

  /** What one crew has connected. Never carries a grant. */
  groupPlugins: (groupId: GroupId) => invoke<Plugin[]>("group_plugins", { groupId }),

  /**
   * Signs the crew in, opening the operator's browser when the server asks for
   * one. Resolves only once they have finished there, which can be minutes, so
   * whatever calls this has to show that it is waiting on a person.
   */
  connectPlugin: (groupId: GroupId, kind: CatalogKind, connection?: string) =>
    invoke<Plugin>("connect_plugin", { groupId, kind, connection: connection ?? null }),

  /**
   * Adds a server the operator addressed themselves, and connects it.
   *
   * The name becomes the prefix every one of its tools is called by, so what
   * comes back on the row is the normalized form rather than what was typed.
   * The key is for a server with no authorization server behind it, which is
   * most of the ones somebody wrote: left out, the server is asked what it
   * wants exactly as a vendor's is.
   *
   * The headers are a third thing and not a third credential: they go on every
   * request whichever of the other two paid for it, which is what makes an
   * `X-API-Key` server and a server behind Cloudflare Access both work.
   */
  addPlugin: (groupId: GroupId, name: string, url: string, key?: string, headers?: HeaderPair[]) =>
    invoke<Plugin>("add_plugin", { groupId, name, url, key: key ?? null, headers }),

  /**
   * Points a crew's own server at a different address, or hands it a new key.
   *
   * A reconnection rather than an edit, because a server at a new address is
   * not the one that published the old tool list. Who may spend it and which of
   * its tools are whose survive, for the reason they survive any reconnection.
   *
   * The headers are the whole set every time, and leaving them out removes
   * them. That is the opposite of the key, whose absence means "ask the
   * server", and the two differ because the panel can read the header names
   * back and cannot read a key back at all.
   */
  readdressPlugin: (
    groupId: GroupId,
    id: PluginId,
    url: string,
    key?: string,
    headers?: HeaderPair[],
  ) => invoke<Plugin>("readdress_plugin", { groupId, id, url, key: key ?? null, headers }),

  /**
   * Dials a server and says what it found, connecting nothing.
   *
   * The whole path a connection runs — both transports, the handshake, the
   * tool list — with whatever has been typed and not yet saved. It stops one
   * step short in one place: a server that wants a sign-in is reported as
   * wanting one rather than sent to a browser.
   */
  probeServer: (url: string, key?: string, headers?: HeaderPair[]) =>
    invoke<ServerReport>("probe_server", { url, key: key ?? null, headers }),

  /**
   * The same question, asked of a plugin this crew already has, using the grant
   * in the store. The only way to find out whether a sign-in is still good
   * without reconnecting, which opens a browser.
   */
  checkPlugin: (id: PluginId) => invoke<ServerReport>("check_plugin", { id }),

  /**
   * Narrows a plugin to named agents, or opens it back up to the crew.
   *
   * The whole answer every time. A merge would let this panel narrow a plugin
   * by forgetting somebody it had not drawn yet.
   */
  setPluginAccess: (id: PluginId, access: PluginAccess) =>
    invoke<Plugin>("set_plugin_access", { id, access }),

  /**
   * Chooses which of a crew's agents may call one of a plugin's tools.
   *
   * One named tool and the whole answer for it, rather than every tool the way
   * `setPluginAccess` takes every agent. Naming the tool is what keeps a panel
   * that can only see today's list from dropping a narrowing filed against one
   * the vendor has stopped publishing; inside the named tool the set of agents
   * is replaced, for the reason it is there.
   */
  setPluginTool: (id: PluginId, tool: string, access: PluginAccess) =>
    invoke<Plugin>("set_plugin_tool", { id, tool, access }),

  disconnectPlugin: (id: PluginId) => invoke<void>("disconnect_plugin", { id }),

  /** The last scan's result. Does not touch the machine, so it is free. */
  agentSignins: (id: AgentId) => invoke<Signin[]>("agent_signins", { id }),

  /**
   * Asks the agent's browser what it is signed in to, right now. Nobody
   * declares these: Chrome is holding the cookies, so the machine is asked.
   * A sleeping or absent machine keeps whatever was last seen.
   */
  scanAgentSignins: (id: AgentId) => invoke<Signin[]>("scan_agent_signins", { id }),

  /**
   * What every recent permission request came to, keyed by id. The requests
   * themselves arrive in the transcript; this is the half that changes.
   */
  approvalStates: () => invoke<Record<ApprovalId, ApprovalState>>("approval_states"),
  /** Every request still waiting on the operator, oldest first. */
  pendingApprovals: () => invoke<Approval[]>("pending_approvals"),

  /** Refused if it was already answered or has lapsed. */
  decideApproval: (id: ApprovalId, decision: Decision) =>
    invoke<Approval>("decide_approval", { id, decision }),
  /** Answers a question with what the operator picked or wrote. */
  answerQuestion: (id: ApprovalId, answer: string) =>
    invoke<Approval>("answer_question", { id, answer }),

  /** What this agent no longer has to ask about. */
  agentGrants: (id: AgentId) => invoke<ProtectedAction[]>("agent_grants", { id }),

  /** Takes one back, and returns what is left. */
  revokeGrant: (id: AgentId, action: ProtectedAction) =>
    invoke<ProtectedAction[]>("revoke_grant", { id, action }),

  listGroups: () => invoke<Group[]>("list_groups"),

  createGroup: (draft: GroupDraft) => invoke<Group>("create_group", { draft }),

  updateGroup: (id: GroupId, draft: GroupDraft) => invoke<Group>("update_group", { id, draft }),

  /**
   * Tests a group's endpoint and key, resolved over the app settings.
   *
   * Sends what is on screen and starts from the stored key, so testing without
   * retyping it reports on the key that is actually there. `id` is null for a
   * group that has not been created yet.
   */
  testGroupConnection: (id: GroupId | null, draft: GroupDraft) =>
    invoke<string>("test_group_connection", { id, draft }),

  /** Refused while the group still holds agents; the error carries which. */
  deleteGroup: (id: GroupId) => invoke<void>("delete_group", { id }),

  /**
   * The group and everybody in it: every agent deleted, every computer and
   * browser destroyed with them. Refused for the default group, before
   * anything is taken.
   */
  disbandGroup: (id: GroupId) => invoke<void>("disband_group", { id }),

  listAgents: () => invoke<AgentCard[]>("list_agents"),

  createAgent: (draft: AgentDraft) => invoke<AgentCard>("create_agent", { draft }),

  updateAgent: (id: AgentId, draft: AgentDraft) => invoke<AgentCard>("update_agent", { id, draft }),

  /**
   * Deletes an agent into the compost, where it waits thirty days.
   *
   * It is out of the rail and unreachable the moment this returns, exactly as a
   * deleted agent has always been. What is different is that its memory, notes,
   * schedule, sign-ins and machines are all still there until the wait is up.
   */
  deleteAgent: (id: AgentId) => invoke<void>("delete_agent", { id }),

  /**
   * Pulls one back out, paused. The name may come back settled: a composted
   * agent frees its name, so somebody hired since may be holding it.
   */
  restoreAgent: (id: AgentId) => invoke<AgentCard>("restore_agent", { id }),

  /** Ends the wait now. Its memory, machines and schedule go for good. */
  purgeAgent: (id: AgentId) => invoke<void>("purge_agent", { id }),

  /**
   * A second agent from the same card: the look, model, skills and
   * instructions. Not the computer, memory, schedule, accounts or transcript,
   * which are what one agent went and did rather than what was written down.
   */
  duplicateAgent: (id: AgentId) => invoke<AgentCard>("duplicate_agent", { id }),

  /**
   * Hires a set of preconfigured agents into one group, in one call.
   *
   * Batched rather than looped for two reasons. Every create announces itself
   * and the rail re-reads the whole roster, so six hires done one at a time
   * redraw the sidebar six times. And a name already taken in the group is
   * settled on the Rust side against the roster *and* the rest of the batch,
   * which is the one place that rule can live without being written twice.
   */
  hireAgents: (groupId: GroupId, drafts: AgentDraft[]) =>
    invoke<AgentCard[]>("hire_agents", { groupId, drafts }),

  setAgentPaused: (id: AgentId, paused: boolean) =>
    invoke<AgentCard>("set_agent_paused", { id, paused }),

  /** Keeps an agent at the top of the rail. Nothing else about it changes. */
  setAgentPinned: (id: AgentId, pinned: boolean) =>
    invoke<AgentCard>("set_agent_pinned", { id, pinned }),

  /**
   * Puts an agent where the operator dropped it: which group, and which place.
   *
   * One call because a drag is one gesture that can be both, and two would
   * leave a state where the agent has joined the group but not landed in it.
   * `before` is the row it goes in front of; `null` is the end of the group.
   */
  moveAgent: (id: AgentId, groupId: GroupId, before: AgentId | null) =>
    invoke<AgentCard>("move_agent", { id, groupId, before }),

  agentActivity: () => invoke<Record<AgentId, Activity>>("agent_activity"),

  agentLastActive: () => invoke<Record<AgentId, number>>("agent_last_active"),

  /** An agent's memory: a small markdown file it maintains for itself. */
  agentMemory: (id: AgentId) => invoke<string>("agent_memory", { id }),

  /** Lets the operator seed or correct an agent's memory by hand. */
  setAgentMemory: (id: AgentId, content: string) =>
    invoke<string>("set_agent_memory", { id, content }),

  /** What an agent is in the middle of, oldest first. */
  agentWorkingNotes: (id: AgentId) => invoke<WorkingNote[]>("agent_working_notes", { id }),

  /** Drops every note an agent holds. The operator's only write here. */
  clearAgentWorkingNotes: (id: AgentId) => invoke<void>("clear_agent_working_notes", { id }),

  /**
   * A channel's newest messages. `through` widens the window until it reaches
   * one particular message, which is what opening a search result needs: a hit
   * from last month is not in the newest three hundred, and a jump that lands
   * somewhere else is a jump that failed.
   */
  channelMessages: (channelId: AgentId, limit?: number, through?: MessageId) =>
    invoke<Envelope[]>("channel_messages", { channelId, limit, through }),

  /**
   * What two agents said to each other. Not a channel read: a send is filed
   * under the recipient and the answer under the sender, so neither of their
   * channels holds the exchange.
   */
  pairMessages: (a: AgentId, b: AgentId, limit?: number) =>
    invoke<Envelope[]>("pair_messages", { a, b, limit }),

  /** The whole conversation, for the activity flow board. */
  conversationFlow: (limit?: number) => invoke<Envelope[]>("conversation_flow", { limit }),

  /**
   * Messages, files, links and routines matching a query.
   *
   * Agents and groups are deliberately not here: this side already holds both
   * to draw the rail, so matching them locally costs no round trip and cannot
   * fall behind the keystroke. `lib/search.ts` puts the two halves in one list.
   */
  search: (query: string, limit?: number) => invoke<SearchHits>("search", { query, limit }),

  /**
   * Takes dropped files into the store, before anything is sent.
   *
   * `paths` are absolute paths from a drop; the bytes never cross IPC, and what
   * comes back is what a message would carry. A file that could not be taken is
   * named in `refused` rather than failing the drop, so one document over the
   * limit does not cost the operator the four beside it.
   */
  stageFiles: (paths: string[]) => invoke<Staged>("stage_files", { paths }),

  /** Copies a file out to the downloads folder, and says where it landed. */
  saveFile: (digest: string, name: string) => invoke<string>("save_file", { digest, name }),

  /**
   * Puts a page an agent wrote somewhere it can be framed, and says where.
   *
   * A round trip rather than a `srcdoc`, because a frame given its markup
   * inline inherits this document's content policy, and this document forbids
   * script. What the returned origin permits is `artifact.rs`'s argument.
   */
  frameArtifact: (html: string) => invoke<{ port: number; id: string }>("frame_artifact", { html }),

  /**
   * Sends what the operator typed, with the files they attached.
   *
   * Only the digest and the name go over: the runtime resolves both against
   * its own store, so the size and the type on the message are read off the
   * disk rather than taken from here.
   */
  sendMessage: (agentId: AgentId, text: string, files: Attachment[] = []) =>
    invoke<RunId>("send_message", {
      agentId,
      text,
      files: files.map(({ digest, name }) => ({ digest, name })),
    }),

  clearChannel: (channelId: AgentId) => invoke<number>("clear_channel", { channelId }),

  /**
   * Sends the message a failed turn was answering again, as a new run. The
   * runtime already retried the call itself; this is the operator's turn.
   */
  retryTurn: (agentId: AgentId, messageId: MessageId) =>
    invoke<RunId>("retry_turn", { agentId, messageId }),

  /**
   * Stops a conversation and everything it set off. False when there was
   * nothing left to stop.
   *
   * A run, not an agent: the thing the operator wants to end reached however
   * many agents it reached, and stopping only the one on screen would leave the
   * rest of the cascade running.
   */
  stopRun: (runId: RunId) => invoke<boolean>("stop_run", { runId }),
  /** Resets a whole group: transcripts, routines, memories and spend. */
  clearGroup: (groupId: GroupId) => invoke<GroupReset>("clear_group", { groupId }),
  agentRoutines: (id: AgentId) => invoke<Routine[]>("agent_routines", { id }),
  createRoutine: (agentId: AgentId, draft: RoutineDraft) =>
    invoke<Routine>("create_routine", { agentId, draft }),
  updateRoutine: (id: RoutineId, draft: RoutineDraft) =>
    invoke<Routine>("update_routine", { id, draft }),

  /** Stops or restarts a routine. Its wording, slot and history all stay. */
  setRoutineActive: (id: RoutineId, active: boolean) =>
    invoke<Routine>("set_routine_active", { id, active }),

  /** Fires it now, exactly as the clock would, without moving the schedule. */
  testRoutine: (id: RoutineId) => invoke<RunId>("test_routine", { id }),

  /** What it has done lately, newest first. */
  routineRuns: (id: RoutineId) => invoke<RoutineRun[]>("routine_runs", { id }),

  deleteRoutine: (id: RoutineId) => invoke<void>("delete_routine", { id }),
  usageSummary: () => invoke<GroupUsage[]>("usage_summary"),
  usageForRuns: (runs: RunId[]) => invoke<RunUsage[]>("usage_for_runs", { runs }),

  getSettings: () => invoke<Settings>("get_settings"),

  updateSettings: (patch: SettingsPatch) => invoke<Settings>("update_settings", { patch }),

  /**
   * Tests what is currently on screen, not what was last saved. Testing the
   * saved config while the operator is looking at an unsaved key reports "no
   * API key configured" for a key they can see, which reads as a bug.
   */
  testConnection: (patch?: SettingsPatch) => invoke<string>("test_connection", { patch }),

  /**
   * The models OpenRouter sees doing one kind of work, most capable first.
   *
   * The category is one of `ROLES` in `lib/roles.ts` and the backend refuses
   * anything else, so this is a use case rather than a URL: the webview does not
   * get to say where the request goes. Cached behind the command for hours, so
   * calling it as an operator types costs one request per use case.
   */
  rankedModels: (category: string) => invoke<RankedModel[]>("ranked_models", { category }),

  subscriptionStatus: () => invoke<SubscriptionStatus>("subscription_status"),

  /** Asks for a code to carry to a browser. Returns in one round trip. */
  beginSubscriptionSignin: () => invoke<DeviceCode>("begin_subscription_signin"),

  /**
   * Waits for the code to be entered, which takes as long as the operator does.
   *
   * Parks for up to fifteen minutes on purpose: the whole sign-in is one call,
   * so abandoning it leaves nothing half-finished to clean up. The caller has to
   * keep its own "waiting" state, because this does not resolve until it is over.
   */
  completeSubscriptionSignin: (code: DeviceCode) =>
    invoke<SubscriptionStatus>("complete_subscription_signin", { code }),

  /** Forgets the sign-in, and moves the provider off it if it was in use. */
  signOutSubscription: () => invoke<Settings>("sign_out_subscription"),

  accountStatus: () => invoke<AccountStatus>("account_status"),

  /**
   * The whole sign-in, in one call.
   *
   * One rather than the subscription's two, because there is no code for the
   * operator to carry: a browser opens, they say yes, and the answer arrives on
   * a port Rust is already listening on. Parks for up to five minutes, so the
   * caller keeps its own "waiting" state; closing the dialog abandons it and
   * leaves nothing behind.
   */
  signInAccount: () => invoke<AccountStatus>("sign_in_account"),

  /** What the account holds, asked of the service rather than remembered. */
  accountConnectors: () => invoke<AccountConnectors>("account_connectors"),

  /** Forgets the sign-in on this machine. */
  signOutAccount: () => invoke<AccountStatus>("sign_out_account"),

  /**
   * Points a crew's plugin at a different authorized identity.
   *
   * Separate from connecting because it is a different act: this moves an
   * existing row to another mailbox and keeps the per-tool switches, where
   * reconnecting would replace the row and lose them.
   */
  setPluginConnection: (groupId: GroupId, kind: CatalogKind, connection: string) =>
    invoke<Plugin>("set_plugin_connection", { groupId, kind, connection }),
};

/**
 * Opens a link in the operating system browser.
 *
 * Agent output can contain links, and following one inside the webview would
 * navigate away from the app with no way back.
 */
export function openExternal(url: string): Promise<void> {
  return openUrl(url);
}

/**
 * Raises an operating system notification, if the operator has ever allowed it.
 *
 * Permission is asked for at the moment the first notification would be shown
 * rather than at launch, so the prompt arrives attached to something the
 * operator can see a reason for.
 *
 * True means it was handed to the operating system, which is not the same as
 * shown. On desktop the permission question is answered yes unconditionally —
 * there is no per-app grant for the plugin to read — so a machine with
 * notifications switched off in System Settings accepts every one of these and
 * displays none. Nothing here can tell the difference, and any copy built on
 * this return value has to be worded so that it does not claim to.
 *
 * Every failure is swallowed. A refused permission, a plugin that is not there,
 * a platform with no notification center: none of them are worth a banner,
 * because the thing being announced is already on screen in the rail and the
 * transcript. This is the redundant copy, not the record.
 */
export async function notifyOperator(title: string, body: string): Promise<boolean> {
  try {
    const granted = (await isPermissionGranted()) || (await requestPermission()) === "granted";
    if (!granted) return false;

    sendNotification({ title, body });
    return true;
  } catch {
    return false;
  }
}

/** Subscribes to runtime events. Returns an unsubscribe function. */
export function onRuntimeEvent(handler: (event: UiEvent) => void): Promise<UnlistenFn> {
  return listen<UiEvent>(EVENT_CHANNEL, (message) => handler(message.payload));
}

/**
 * Subscribes to the menu bar asking for a channel to be opened.
 *
 * The window is already shown, unminimized and focused by the time this
 * arrives; all that is left is which channel it lands in. Answering a
 * permission request from the strip does not come through here: that one is
 * decided in Rust and reaches the transcript as an ordinary settled event.
 */
export function onRevealRequest(handler: (agent: AgentId) => void): Promise<UnlistenFn> {
  return listen<AgentId>(REVEAL_CHANNEL, (message) => handler(message.payload));
}

/**
 * Files dragged onto the window.
 *
 * Tauri hands over paths rather than bytes, which is the whole reason
 * `dragDropEnabled` is on: a dropped document is read by the Rust side and
 * never enters the renderer. `over` fires while a drag is above the window and
 * is what the drop target highlights on.
 */
export async function onFileDrop(handlers: {
  dropped: (paths: string[]) => void;
  over: (inside: boolean) => void;
}): Promise<UnlistenFn> {
  const stops = await Promise.all([
    listen(TauriEvent.DRAG_ENTER, () => handlers.over(true)),
    listen(TauriEvent.DRAG_LEAVE, () => handlers.over(false)),
    listen<{ paths: string[] }>(TauriEvent.DRAG_DROP, (message) => {
      handlers.over(false);
      handlers.dropped(message.payload.paths ?? []);
    }),
  ]);
  return () => {
    for (const stop of stops) stop();
  };
}
