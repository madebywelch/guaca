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
  Computer,
  Envelope,
  Group,
  GroupDraft,
  GroupId,
  RunId,
  Settings,
  SettingsPatch,
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

  /** An agent's notes: a small markdown file it maintains for itself. */
  agentNotes: (id: AgentId) => invoke<string>("agent_notes", { id }),

  /** Lets the operator seed or correct an agent's notes by hand. */
  setAgentNotes: (id: AgentId, content: string) =>
    invoke<string>("set_agent_notes", { id, content }),

  channelMessages: (channelId: AgentId, limit?: number) =>
    invoke<Envelope[]>("channel_messages", { channelId, limit }),

  /** The whole conversation, for the activity flow board. */
  conversationFlow: (limit?: number) => invoke<Envelope[]>("conversation_flow", { limit }),

  sendMessage: (agentId: AgentId, text: string) => invoke<RunId>("send_message", { agentId, text }),

  clearChannel: (channelId: AgentId) => invoke<number>("clear_channel", { channelId }),
  /** Empties every channel in a group. Returns how many messages went. */
  clearGroup: (groupId: GroupId) => invoke<number>("clear_group", { groupId }),

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
