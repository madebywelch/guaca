/**
 * Typed wrappers over the Tauri command surface.
 *
 * Every call the UI can make goes through here, so the set of things the
 * webview is able to do is one readable list. Tauri maps camelCase argument
 * keys onto the Rust snake_case parameters.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";

import type {
  Activity,
  AgentCard,
  AgentDraft,
  AgentId,
  Approval,
  ApprovalId,
  ApprovalState,
  Computer,
  Connector,
  ConnectorDraft,
  ConnectorId,
  Decision,
  Envelope,
  Group,
  GroupDraft,
  GroupId,
  GroupReset,
  GroupUsage,
  MessageId,
  ProtectedAction,
  Routine,
  RoutineDraft,
  RoutineId,
  RunId,
  RunUsage,
  Settings,
  SettingsPatch,
  Signin,
  UiEvent,
} from "./types";

const EVENT_CHANNEL = "guac://event";

export const api = {
  /** `null` when the agent has never been given a computer. */
  agentComputer: (id: AgentId) => invoke<Computer | null>("agent_computer", { id }),

  /** Creates or wakes the sandbox, and brings the desktop up. Idempotent. */
  startAgentComputer: (id: AgentId) => invoke<Computer>("start_agent_computer", { id }),

  /** Puts it to sleep. The disk is kept, so a signed-in browser stays signed in. */
  stopAgentComputer: (id: AgentId) => invoke<Computer | null>("stop_agent_computer", { id }),

  /** Destroys the sandbox and everything on its disk. */
  deleteAgentComputer: (id: AgentId) => invoke<void>("delete_agent_computer", { id }),

  /** Every account a crew can reach. Never carries a credential's value. */
  groupConnectors: (groupId: GroupId) => invoke<Connector[]>("group_connectors", { groupId }),

  createConnector: (draft: ConnectorDraft) => invoke<Connector>("create_connector", { draft }),

  deleteConnector: (id: ConnectorId) => invoke<void>("delete_connector", { id }),

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

  /** Refused if it was already answered or has lapsed. */
  decideApproval: (id: ApprovalId, decision: Decision) =>
    invoke<Approval>("decide_approval", { id, decision }),

  /** What this agent no longer has to ask about. */
  agentGrants: (id: AgentId) => invoke<ProtectedAction[]>("agent_grants", { id }),

  /** Takes one back, and returns what is left. */
  revokeGrant: (id: AgentId, action: ProtectedAction) =>
    invoke<ProtectedAction[]>("revoke_grant", { id, action }),

  listGroups: () => invoke<Group[]>("list_groups"),

  createGroup: (draft: GroupDraft) => invoke<Group>("create_group", { draft }),

  updateGroup: (id: GroupId, draft: GroupDraft) => invoke<Group>("update_group", { id, draft }),

  /** Refused while the group still holds agents; the error carries which. */
  deleteGroup: (id: GroupId) => invoke<void>("delete_group", { id }),

  listAgents: () => invoke<AgentCard[]>("list_agents"),

  createAgent: (draft: AgentDraft) => invoke<AgentCard>("create_agent", { draft }),

  updateAgent: (id: AgentId, draft: AgentDraft) => invoke<AgentCard>("update_agent", { id, draft }),

  deleteAgent: (id: AgentId) => invoke<void>("delete_agent", { id }),

  setAgentPaused: (id: AgentId, paused: boolean) =>
    invoke<AgentCard>("set_agent_paused", { id, paused }),

  agentActivity: () => invoke<Record<AgentId, Activity>>("agent_activity"),

  agentLastActive: () => invoke<Record<AgentId, number>>("agent_last_active"),

  /** An agent's memory: a small markdown file it maintains for itself. */
  agentNotes: (id: AgentId) => invoke<string>("agent_notes", { id }),

  /** Lets the operator seed or correct an agent's memory by hand. */
  setAgentNotes: (id: AgentId, content: string) =>
    invoke<string>("set_agent_notes", { id, content }),

  channelMessages: (channelId: AgentId, limit?: number) =>
    invoke<Envelope[]>("channel_messages", { channelId, limit }),

  /** The whole conversation, for the activity flow board. */
  conversationFlow: (limit?: number) => invoke<Envelope[]>("conversation_flow", { limit }),

  sendMessage: (agentId: AgentId, text: string) => invoke<RunId>("send_message", { agentId, text }),

  clearChannel: (channelId: AgentId) => invoke<number>("clear_channel", { channelId }),

  /**
   * Sends the message a failed turn was answering again, as a new run. The
   * runtime already retried the call itself; this is the operator's turn.
   */
  retryTurn: (agentId: AgentId, messageId: MessageId) =>
    invoke<RunId>("retry_turn", { agentId, messageId }),
  /** Resets a whole group: transcripts, routines, memories and spend. */
  clearGroup: (groupId: GroupId) => invoke<GroupReset>("clear_group", { groupId }),
  agentRoutines: (id: AgentId) => invoke<Routine[]>("agent_routines", { id }),
  createRoutine: (agentId: AgentId, draft: RoutineDraft) =>
    invoke<Routine>("create_routine", { agentId, draft }),
  updateRoutine: (id: RoutineId, draft: RoutineDraft) =>
    invoke<Routine>("update_routine", { id, draft }),
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

/** Subscribes to runtime events. Returns an unsubscribe function. */
export function onRuntimeEvent(handler: (event: UiEvent) => void): Promise<UnlistenFn> {
  return listen<UiEvent>(EVENT_CHANNEL, (message) => handler(message.payload));
}
