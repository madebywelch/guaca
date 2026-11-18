import { useCallback, useMemo } from "react";
import { create } from "zustand";
import { diagnostic } from "./diagnostic";
import { api } from "./ipc";
import { forgetRestart, type Link, restarting } from "./link";
import { loadPrefs, type Prefs, savePrefs } from "./prefs";
import { type DropTarget, landsBefore, railOrder } from "./rail";
import { hosted, isUnreachable, openExternal, PAGE_ID } from "./transport";

let decisionRead = 0;
let rosterRead = 0;
let usageRead = 0;
let approvalRead = 0;
let escalationRead = 0;
let outcomeRead = 0;
let runningRead = 0;
const channelReads = new Map<ChannelKey, { arrivals: Envelope[] }>();

const EVERYTHING: Capabilities = {
  loopbackEndpoints: true,
  claudeProvider: true,
  localFiles: true,
  borrowedSignins: true,
};

import { UNSEEN_LIMIT } from "./outcomes";
import { keepThought } from "./reasoning";
import type { LiveCall } from "./trail";
import type {
  Activity,
  AgentCard,
  AgentId,
  Approval,
  ApprovalId,
  ApprovalState,
  ArtifactId,
  Capabilities,
  Decision,
  Envelope,
  Escalation,
  EscalationId,
  Group,
  GroupId,
  GroupUsage,
  MessageId,
  Outcome,
  Participant,
  RoutineId,
  RunId,
  Running,
  Settings,
  Tokens,
  UiEvent,
  WorkDecision,
} from "./types";
import { asSentence, errorMessage, skillScopeKey } from "./types";

export type ChannelKey = AgentId;

export interface StreamBuffer {
  channelId: AgentId;
  agentId: AgentId;
  text: string;
  truncated?: boolean;
  to: Participant;
}

export interface Pulse {
  id: number;
  from: AgentId;
  to: AgentId;
  color: string;
}

export const PULSE_WINDOW_MS = 90_000;

export interface Banner {
  tone: "error" | "info" | "ok";
  text: string;
  fix?: "host";
}

export interface Placement {
  groupId: GroupId;
  before: AgentId | null;
  pinned?: boolean;
}

export interface State {
  agents: AgentCard[];
  groups: Group[];
  activity: Record<AgentId, Activity>;
  lastActive: Record<AgentId, number>;
  settings: Settings | null;
  capabilities: Capabilities;
  prefs: Prefs;
  activeRun: Record<AgentId, RunId | undefined>;

  usage: Record<GroupId, Tokens | undefined>;
  pulse: Record<GroupId, { at: number; tokens: number }[] | undefined>;

  approvals: Record<ApprovalId, ApprovalState | undefined>;

  answers: Record<ApprovalId, string | undefined>;

  pending: Approval[];

  stuck: Escalation[];
  decisions: WorkDecision[];
  decisionError: string | null;
  outcomes: Outcome[];
  refreshOutcomes: (valid?: () => boolean) => Promise<void>;
  running: Running[];
  refreshRunning: (valid?: () => boolean) => Promise<void>;
  seeOutcomes: (target: { runIds?: RunId[]; before?: number }) => Promise<void>;
  forYou: boolean;
  forYouAsked: number;
  showForYou: (open: boolean) => void;
  artifactsOpen: { id: ArtifactId | null } | null;
  showArtifacts: (open: { id: ArtifactId | null } | null) => void;
  refreshDecisions: (valid?: () => boolean) => Promise<void>;

  selected: ChannelKey | null;
  railGroup: GroupId | null;
  messages: Record<ChannelKey, Envelope[] | undefined>;
  streams: Record<MessageId, StreamBuffer | undefined>;
  reasoning: Record<AgentId, string | undefined>;
  trail: Record<AgentId, LiveCall[] | undefined>;
  finishedAt: Record<AgentId, number | undefined>;
  pulses: Pulse[];

  focused: MessageId | null;

  openingRoutine: RoutineId | null;

  routineVersion: Record<AgentId, number | undefined>;

  calendarVersion: number;

  artifactsVersion: number;

  memoryVersion: Record<AgentId, number | undefined>;
  workingNotesVersion: Record<AgentId, number | undefined>;
  notebookVersion: Record<AgentId, number | undefined>;
  skillsVersion: Record<string, number | undefined>;

  banner: Banner | null;
  link: Link;
  lastHeard: number | null;
  handoff: string | null;
  sessionSpend: Tokens;

  bootstrap: (valid?: () => boolean) => Promise<void>;
  resynchronize: (valid?: () => boolean) => Promise<void>;
  refreshAgents: (valid?: () => boolean) => Promise<void>;
  refreshUsage: (valid?: () => boolean) => Promise<void>;
  refreshApprovals: (valid?: () => boolean) => Promise<void>;
  refreshEscalations: (valid?: () => boolean) => Promise<void>;
  clearEscalation: (id: EscalationId) => Promise<void>;
  select: (key: ChannelKey, valid?: () => boolean) => Promise<void>;
  focusGroup: (id: GroupId | null) => Promise<void>;
  leaveChannel: () => void;
  moveAgent: (id: AgentId, at: Placement) => Promise<void>;
  dropAgent: (id: AgentId, target: DropTarget) => Promise<void>;
  dropGroup: (id: GroupId, onto: GroupId) => Promise<void>;
  loadChannel: (key: ChannelKey, through?: MessageId, valid?: () => boolean) => Promise<void>;
  openMessage: (channel: AgentId, message: MessageId) => Promise<void>;
  clearFocus: () => void;
  openRoutine: (id: RoutineId) => void;
  routineOpened: () => void;
  decideApproval: (id: ApprovalId, decision: Decision) => Promise<void>;
  answerQuestion: (id: ApprovalId, answer: string) => Promise<void>;
  settle: (answer: () => Promise<unknown>) => Promise<void>;
  applyEvent: (event: UiEvent, valid?: () => boolean) => void;
  dismissPulse: (id: number) => void;
  setBanner: (banner: Banner | null) => void;
  linkDown: (error: unknown, heardAt: number) => void;
  linkReconciling: (heardAt: number) => void;
  linkUp: (forget?: boolean) => void;
  showFailure: (error: unknown) => void;
  setHandoff: (url: string | null) => void;
  setSettings: (settings: Settings) => void;
  setPrefs: (patch: Partial<Prefs>) => void;
}

let pulseSeq = 0;

function failureBanner(error: unknown): Banner {
  return {
    tone: "error",
    text: asSentence(errorMessage(error)),
    ...(isUnreachable(error) ? { fix: "host" as const } : {}),
  };
}

function byGroup(rows: GroupUsage[]): Record<GroupId, Tokens> {
  const out: Record<GroupId, Tokens> = {};
  for (const row of rows) {
    out[row.groupId] = {
      prompt: row.prompt,
      completion: row.completion,
      cost: row.cost,
      calls: row.calls,
    };
  }
  return out;
}

function insert(existing: Envelope[] | undefined, message: Envelope): Envelope[] | undefined {
  if (existing === undefined) return undefined;
  if (existing.some((m) => m.id === message.id)) return existing;

  const next = [...existing, message];
  next.sort((a, b) => a.createdAt - b.createdAt || (a.id < b.id ? -1 : 1));
  return next;
}

function keptFocus(state: State, key: ChannelKey): GroupId | null {
  if (state.railGroup === null) return state.railGroup;
  const agent = state.agents.find((a) => a.id === key);
  return agent ? agent.groupId : state.railGroup;
}

function keptChannel(state: State, group: GroupId | null): boolean {
  const key = state.selected;
  if (group === null || key === null) return true;
  const agent = state.agents.find((a) => a.id === key);
  return agent === undefined || agent.groupId === group;
}

export const useStore = create<State>((set, get) => ({
  agents: [],
  groups: [],
  activity: {},
  lastActive: {},
  settings: null,
  capabilities: EVERYTHING,
  prefs: loadPrefs(),
  activeRun: {},
  usage: {},
  pulse: {},
  approvals: {},
  answers: {},
  pending: [],
  stuck: [],
  decisions: [],
  decisionError: null,
  outcomes: [],
  running: [],
  async refreshRunning(valid = () => true) {
    const read = ++runningRead;
    try {
      const running = await api.runningRuns();
      if (valid() && read === runningRead) set({ running });
    } catch (error) {
      console.warn(`Running work could not be read: ${errorMessage(error)}`);
    }
  },
  async refreshOutcomes(valid = () => true) {
    const read = ++outcomeRead;
    try {
      const outcomes = await api.listOutcomes({ unseenOnly: true, limit: UNSEEN_LIMIT });
      if (valid() && read === outcomeRead) set({ outcomes });
    } catch (error) {
      console.warn(`Finished work could not be read: ${errorMessage(error)}`);
    }
  },
  async seeOutcomes(target) {
    try {
      const marked = new Set(await api.markOutcomesSeen(target));
      set((state) => ({ outcomes: state.outcomes.filter((one) => !marked.has(one.runId)) }));
    } catch (error) {
      get().showFailure(error);
    }
  },
  forYou: false,
  forYouAsked: 0,
  showForYou: (open) =>
    set((state) =>
      open ? { forYou: true, forYouAsked: state.forYouAsked + 1 } : { forYou: false },
    ),
  artifactsOpen: null,
  showArtifacts: (open) => set({ artifactsOpen: open }),
  async refreshDecisions(valid = () => true) {
    const read = ++decisionRead;
    try {
      const decisions = await api.listDecisions();
      if (valid() && read === decisionRead) set({ decisions, decisionError: null });
    } catch (error) {
      if (valid() && read === decisionRead) set({ decisionError: errorMessage(error) });
    }
  },
  selected: null,
  railGroup: null,
  messages: {},
  streams: {},
  reasoning: {},
  trail: {},
  finishedAt: {},
  pulses: [],
  focused: null,
  openingRoutine: null,
  routineVersion: {},
  calendarVersion: 0,
  artifactsVersion: 0,
  memoryVersion: {},
  workingNotesVersion: {},
  skillsVersion: {},
  notebookVersion: {},
  banner: null,
  link: "up",
  lastHeard: null,
  handoff: null,
  sessionSpend: { prompt: 0, completion: 0, cost: null, calls: 0 },

  async bootstrap(valid = () => true) {
    const decisionSnapshot = ++decisionRead;
    const rosterSnapshot = ++rosterRead;
    const usageSnapshot = ++usageRead;
    const approvalSnapshot = ++approvalRead;
    const escalationSnapshot = ++escalationRead;
    const [
      agents,
      groups,
      activity,
      lastActive,
      settings,
      capabilities,
      usage,
      approvals,
      answers,
      pending,
      stuck,
      decisions,
    ] = await Promise.all([
      api.listAgents(),
      api.listGroups(),
      hosted ? Promise.resolve(null) : api.agentActivity(),
      api.agentLastActive(),
      api.getSettings(),
      api.capabilities(),
      api.usageSummary(),
      api.approvalStates(),
      api.approvalAnswers(),
      api.pendingApprovals(),
      api.openEscalations(),
      api.listDecisions(),
      get().refreshOutcomes(valid),
      get().refreshRunning(valid),
    ]);
    if (!valid()) return;
    set({
      ...(rosterSnapshot === rosterRead ? { agents, groups } : {}),
      ...(activity ? { activity } : {}),
      lastActive,
      settings,
      capabilities,
      ...(usageSnapshot === usageRead ? { usage: byGroup(usage) } : {}),
      ...(approvalSnapshot === approvalRead ? { approvals, answers, pending } : {}),
      ...(escalationSnapshot === escalationRead ? { stuck } : {}),
      ...(decisionSnapshot === decisionRead ? { decisions, decisionError: null } : {}),
    });
  },

  async resynchronize(valid = () => true) {
    await get().bootstrap(valid);
    if (!valid()) return;
    const { selected, agents } = get();
    const live = agents.filter((a) => a.lifecycle !== "terminated");
    const next = live.some((a) => a.id === selected) ? selected : null;
    set((state) => ({
      selected: next,
      railGroup: state.groups.some((g) => g.id === state.railGroup) ? state.railGroup : null,
      routineVersion: Object.fromEntries(
        agents.map((a) => [a.id, (state.routineVersion[a.id] ?? 0) + 1]),
      ),
      memoryVersion: Object.fromEntries(
        agents.map((a) => [a.id, (state.memoryVersion[a.id] ?? 0) + 1]),
      ),
      workingNotesVersion: Object.fromEntries(
        agents.map((a) => [a.id, (state.workingNotesVersion[a.id] ?? 0) + 1]),
      ),
    }));
    if (next) await get().loadChannel(next, undefined, valid);
  },

  async refreshAgents(valid = () => true) {
    const rosterSnapshot = ++rosterRead;
    const [agents, groups] = await Promise.all([api.listAgents(), api.listGroups()]);
    if (!valid() || rosterSnapshot !== rosterRead) return;
    set((state) => ({
      agents,
      groups,
      railGroup: groups.some((g) => g.id === state.railGroup) ? state.railGroup : null,
    }));

    const selected = get().selected;
    if (selected && !agents.some((a) => a.id === selected && a.lifecycle !== "terminated")) {
      set({ selected: null, focused: null });
    }
  },

  async select(key, valid = () => true) {
    diagnostic("select.requested", key);
    if (!valid()) {
      diagnostic("select.invalid", key);
      return;
    }
    set((state) => ({
      selected: key,
      focused: null,
      openingRoutine: null,
      railGroup: keptFocus(state, key),
    }));
    diagnostic("select.committed", get().selected);
    await get().loadChannel(key, undefined, valid);
  },

  async focusGroup(id) {
    const closing = !keptChannel(get(), id);
    set({ railGroup: id, ...(closing ? { selected: null, focused: null } : {}) });
  },

  leaveChannel() {
    set({ selected: null, focused: null });
  },

  async moveAgent(id, at) {
    const current = get().agents.find((a) => a.id === id);
    if (at.pinned !== undefined && current && current.pinned !== at.pinned) {
      await api.setAgentPinned(id, at.pinned);
    }
    await api.moveAgent(id, at.groupId, at.before);
    await get().refreshAgents();
  },

  async dropAgent(id, target) {
    const state = get();
    const dragged = state.agents.find((a) => a.id === id);
    if (!dragged) return;

    const live = state.agents.filter((a) => a.lifecycle !== "terminated");

    if (target.kind === "group") {
      await get().moveAgent(id, { groupId: target.id, before: null });
      return;
    }

    const onto = live.find((a) => a.id === target.id);
    if (!onto || onto.id === id) return;

    const section = live.filter((a) => a.groupId === onto.groupId && a.pinned === onto.pinned);
    const order = railOrder(section, {
      activity: state.activity,
      lastActive: state.lastActive,
      frozen: true,
    });
    const before = landsBefore(order, id, onto.id);
    if (before === undefined) return;
    await get().moveAgent(id, { groupId: onto.groupId, before, pinned: onto.pinned });
  },

  async dropGroup(id, onto) {
    const { groups } = get();
    if (!groups.some((g) => g.id === id)) return;
    const before = landsBefore(groups, id, onto);
    if (before === undefined) return;
    set({ groups: await api.moveGroup(id, before) });
  },

  async loadChannel(key, through, valid = () => true) {
    const read = { arrivals: channelReads.get(key)?.arrivals ?? [] };
    channelReads.set(key, read);
    const before = new Set((get().messages[key] ?? []).map((m) => m.id));
    try {
      const messages = await api.channelMessages(key, 300, through);
      if (!valid() || channelReads.get(key) !== read) return;
      set((state) => {
        let merged = messages;
        for (const arrived of [...read.arrivals, ...(state.messages[key] ?? [])]) {
          if (!before.has(arrived.id)) merged = insert(merged, arrived) ?? merged;
        }
        return { messages: { ...state.messages, [key]: merged } };
      });
    } finally {
      if (channelReads.get(key) === read) channelReads.delete(key);
    }
  },

  async openMessage(channel, message) {
    set((state) => ({
      selected: channel,
      focused: message,
      railGroup: keptFocus(state, channel),
    }));
    await get().loadChannel(channel, message);
  },

  clearFocus() {
    set({ focused: null });
  },

  openRoutine(id) {
    set({ openingRoutine: id });
  },

  routineOpened() {
    set({ openingRoutine: null });
  },

  async refreshUsage(valid = () => true) {
    const read = ++usageRead;
    const usage = await api.usageSummary();
    if (valid() && read === usageRead) set({ usage: byGroup(usage) });
  },

  async refreshApprovals(valid = () => true) {
    const read = ++approvalRead;
    const [approvals, answers, pending] = await Promise.all([
      api.approvalStates(),
      api.approvalAnswers(),
      api.pendingApprovals(),
    ]);
    if (valid() && read === approvalRead) set({ approvals, answers, pending });
  },

  async refreshEscalations(valid = () => true) {
    const read = ++escalationRead;
    const stuck = await api.openEscalations();
    if (valid() && read === escalationRead) set({ stuck });
  },

  async clearEscalation(id) {
    try {
      await api.clearEscalation(id);
    } catch (error) {
      get().showFailure(error);
    }
    await get().refreshEscalations();
  },

  async decideApproval(id, decision) {
    await get().settle(() => api.decideApproval(id, decision));
  },

  async answerQuestion(id, answer) {
    await get().settle(() => api.answerQuestion(id, answer));
  },

  async settle(answer) {
    try {
      await answer();
    } catch (error) {
      get().showFailure(error);
    }
    await get().refreshApprovals();
  },

  applyEvent(event, valid = () => true) {
    switch (event.type) {
      case "decisionsChanged":
        void get().refreshDecisions(valid);
        return;
      case "decisionReminder":
        void get().refreshDecisions(valid);
        return;
      case "agentsChanged": {
        void get().refreshAgents(valid);
        void get().refreshDecisions(valid);
        void get().refreshOutcomes(valid);
        break;
      }

      case "outcomeRecorded":
        void get().refreshOutcomes(valid);
        return;

      case "outcomesSeen": {
        const seen = new Set(event.runIds);
        set((state) => ({ outcomes: state.outcomes.filter((one) => !seen.has(one.runId)) }));
        void get().refreshOutcomes(valid);
        return;
      }

      case "liveSnapshot": {
        set({
          activity: event.activity,
          streams: Object.fromEntries(
            Object.entries(event.streams).map(([id, stream]) => [
              id,
              stream.truncated ? { ...stream, text: "" } : stream,
            ]),
          ),
          activeRun: Object.fromEntries(
            Object.values(event.streams).map((s) => [s.agentId, s.runId]),
          ),
          reasoning: {},
          trail: {},
          pulse: {},
          pulses: [],
          handoff: null,
        });
        void get().refreshRunning(valid);
        break;
      }

      case "openUrl": {
        if (event.client && event.client !== PAGE_ID) break;
        set({ handoff: event.url });
        void openExternal(event.url);
        break;
      }

      case "messageAppended": {
        const message = event.message;
        if (get().messages[message.channelId] === undefined) {
          channelReads.get(message.channelId)?.arrivals.push(message);
        }
        set((state) => {
          const messages = { ...state.messages };
          messages[message.channelId] = insert(messages[message.channelId], message);

          const from = message.from;
          const to = message.to;
          let pulses = state.pulses;
          if (from.kind === "agent" && to.kind === "agent") {
            const sender = state.agents.find((a) => a.id === from.id);
            pulses = [
              ...state.pulses,
              { id: ++pulseSeq, from: from.id, to: to.id, color: sender?.color ?? "#c7d96b" },
            ];
          }

          const lastActive = { ...state.lastActive };
          for (const end of [from, to]) {
            if (end.kind === "agent") {
              lastActive[end.id] = Math.max(lastActive[end.id] ?? 0, message.createdAt);
            }
          }

          return { messages, pulses, lastActive };
        });
        break;
      }

      case "streamStarted": {
        set((state) => {
          const reasoning = { ...state.reasoning };
          delete reasoning[event.agentId];
          const trail = { ...state.trail };
          delete trail[event.agentId];
          return {
            reasoning,
            trail,
            activeRun: { ...state.activeRun, [event.agentId]: event.runId },
            streams: {
              ...state.streams,
              [event.messageId]: {
                channelId: event.channelId,
                agentId: event.agentId,
                text: "",
                to: event.to,
              },
            },
          };
        });
        break;
      }

      case "streamDelta": {
        set((state) => {
          const current = state.streams[event.messageId];
          if (!current || current.truncated) return state;
          return {
            streams: {
              ...state.streams,
              [event.messageId]: { ...current, text: current.text + event.text },
            },
          };
        });
        break;
      }

      case "reasoningDelta": {
        set((state) => {
          const stream = state.streams[event.messageId];
          if (!stream) return state;
          const before = state.reasoning[stream.agentId] ?? "";
          const next = keepThought(before, event.text);
          const dropped = before.length + event.text.length - next.length;
          return {
            reasoning: { ...state.reasoning, [stream.agentId]: next },
            ...(dropped > 0 && state.trail[stream.agentId]
              ? {
                  trail: {
                    ...state.trail,
                    [stream.agentId]: state.trail[stream.agentId]!.map((call) => ({
                      ...call,
                      reasoningOffset: Math.max(0, (call.reasoningOffset ?? 0) - dropped),
                    })),
                  },
                }
              : {}),
          };
        });
        break;
      }

      case "toolStarted": {
        set((state) => {
          const stream = state.streams[event.messageId];
          if (!stream) return state;
          const call: LiveCall = {
            callId: event.callId,
            name: event.name,
            arguments: event.arguments,
            done: null,
            startedAt: Date.now(),
            reasoningOffset: state.reasoning[stream.agentId]?.length ?? 0,
          };
          return {
            trail: {
              ...state.trail,
              [stream.agentId]: [...(state.trail[stream.agentId] ?? []), call],
            },
          };
        });
        break;
      }

      case "toolFinished": {
        set((state) => {
          const stream = state.streams[event.messageId];
          const held = stream && state.trail[stream.agentId];
          if (!stream || !held) return state;
          return {
            trail: {
              ...state.trail,
              [stream.agentId]: held.map((call) =>
                call.callId === event.callId ? { ...call, done: event.part } : call,
              ),
            },
          };
        });
        break;
      }

      case "streamEnded": {
        set((state) => {
          const streams = { ...state.streams };
          const ending = streams[event.messageId];
          delete streams[event.messageId];
          if (!ending) return { streams };

          const reasoning = { ...state.reasoning };
          delete reasoning[ending.agentId];
          const trail = { ...state.trail };
          delete trail[ending.agentId];
          return {
            streams,
            reasoning,
            trail,
            finishedAt: { ...state.finishedAt, [ending.agentId]: Date.now() },
          };
        });
        break;
      }

      case "activityChanged": {
        set((state) => {
          const activeRun =
            event.activity.state === "idle" && state.activeRun[event.agentId] !== undefined
              ? (() => {
                  const next = { ...state.activeRun };
                  delete next[event.agentId];
                  return next;
                })()
              : state.activeRun;

          return { activity: { ...state.activity, [event.agentId]: event.activity }, activeRun };
        });
        void get().refreshRunning(valid);
        break;
      }

      case "channelsCleared": {
        const emptied = new Set<ChannelKey>(event.agents);
        for (const key of emptied) channelReads.delete(key);
        set((state) => {
          const messages = { ...state.messages };
          for (const key of Object.keys(messages) as ChannelKey[]) {
            if (emptied.has(key)) delete messages[key];
          }
          return { messages };
        });

        const open = get().selected;
        if (open && emptied.has(open)) {
          void get().loadChannel(open, undefined, valid);
        }
        set((state) => {
          const memory = { ...state.memoryVersion };
          const working = { ...state.workingNotesVersion };
          for (const agent of event.agents) {
            memory[agent] = (memory[agent] ?? 0) + 1;
            working[agent] = (working[agent] ?? 0) + 1;
          }
          return { memoryVersion: memory, workingNotesVersion: working };
        });
        void get().refreshUsage(valid);
        break;
      }

      case "tokensUsed": {
        const spent = event.prompt + event.completion;
        set((state) => {
          const held = state.usage[event.groupId] ?? {
            prompt: 0,
            completion: 0,
            cost: null,
            calls: 0,
          };
          const cutoff = Date.now() - PULSE_WINDOW_MS;
          const recent = (state.pulse[event.groupId] ?? []).filter((p) => p.at >= cutoff);
          const session = state.sessionSpend;
          return {
            sessionSpend: {
              prompt: session.prompt + event.prompt,
              completion: session.completion + event.completion,
              cost:
                event.cost === null && session.cost === null
                  ? null
                  : (session.cost ?? 0) + (event.cost ?? 0),
              calls: session.calls + 1,
            },
            usage: {
              ...state.usage,
              [event.groupId]: {
                prompt: held.prompt + event.prompt,
                completion: held.completion + event.completion,
                cost: event.cost === null ? held.cost : (held.cost ?? 0) + event.cost,
                calls: held.calls + 1,
              },
            },
            pulse: {
              ...state.pulse,
              [event.groupId]: [...recent, { at: Date.now(), tokens: spent }],
            },
          };
        });
        break;
      }

      case "runSettled": {
        set((state) => {
          const activeRun = { ...state.activeRun };
          let changed = false;
          for (const [agent, run] of Object.entries(activeRun)) {
            if (run === event.runId) {
              delete activeRun[agent as AgentId];
              changed = true;
            }
          }
          return changed ? { activeRun } : {};
        });
        void get().refreshUsage(valid);
        void get().refreshRunning(valid);
        break;
      }

      case "approvalRequested": {
        set((state) => ({
          approvals: { ...state.approvals, [event.approvalId]: "pending" },
        }));
        void get().refreshApprovals(valid);
        break;
      }

      case "approvalSettled": {
        set((state) => ({
          approvals: { ...state.approvals, [event.approvalId]: event.state },
        }));
        void get().refreshApprovals(valid);
        break;
      }

      case "escalationRaised":
      case "escalationCleared": {
        void get().refreshEscalations(valid);
        break;
      }

      case "calendarChanged": {
        set((state) => ({ calendarVersion: state.calendarVersion + 1 }));
        break;
      }

      case "artifactsChanged": {
        set((state) => ({ artifactsVersion: state.artifactsVersion + 1 }));
        break;
      }

      case "routinesChanged": {
        set((state) => ({
          routineVersion: {
            ...state.routineVersion,
            [event.agentId]: (state.routineVersion[event.agentId] ?? 0) + 1,
          },
        }));
        break;
      }

      case "memoryChanged": {
        set((state) => ({
          memoryVersion: {
            ...state.memoryVersion,
            [event.agentId]: (state.memoryVersion[event.agentId] ?? 0) + 1,
          },
        }));
        break;
      }

      case "workingNotesChanged": {
        set((state) => ({
          workingNotesVersion: {
            ...state.workingNotesVersion,
            [event.agentId]: (state.workingNotesVersion[event.agentId] ?? 0) + 1,
          },
        }));
        void get().refreshRunning(valid);
        break;
      }

      case "settingsChanged": {
        set({ settings: event.settings });
        break;
      }

      case "notebookChanged": {
        set((state) => ({
          notebookVersion: {
            ...state.notebookVersion,
            [event.agentId]: (state.notebookVersion[event.agentId] ?? 0) + 1,
          },
        }));
        break;
      }

      case "skillsChanged": {
        const key = skillScopeKey(event.scope);
        set((state) => ({
          skillsVersion: { ...state.skillsVersion, [key]: (state.skillsVersion[key] ?? 0) + 1 },
        }));
        break;
      }
    }
  },

  dismissPulse(id) {
    set((state) => ({ pulses: state.pulses.filter((p) => p.id !== id) }));
  },

  setBanner(banner) {
    set({ banner });
  },

  showFailure(error) {
    set({ banner: failureBanner(error) });
  },

  linkDown(error, heardAt) {
    if (restarting()) return;
    set({ banner: failureBanner(error), link: "down", lastHeard: heardAt });
  },

  linkReconciling(heardAt) {
    set({ link: "down", lastHeard: heardAt });
  },

  linkUp(forget = true) {
    const { link, banner } = get();
    if (forget) forgetRestart();
    if (link === "up" && banner?.fix !== "host") return;
    set({
      link: "up",
      lastHeard: null,
      ...(banner?.fix === "host" ? { banner: null } : {}),
    });
  },

  setHandoff(handoff) {
    set({ handoff });
  },

  setSettings(settings) {
    set({ settings });
  },

  setPrefs(patch) {
    set((state) => {
      const prefs = { ...state.prefs, ...patch };
      savePrefs(prefs);
      return { prefs };
    });
  },
}));

export function forYouCount(state: State): number {
  const decisions = state.decisions.filter(
    (item) => item.status === "pending" || (item.status === "answered" && item.interrupted),
  ).length;
  return decisions + state.pending.length + state.stuck.length;
}

export function useLiveAgents(): AgentCard[] {
  const agents = useStore((s) => s.agents);

  return useMemo(() => {
    return agents
      .filter((a) => a.lifecycle !== "terminated")
      .map((agent, order) => ({ agent, order }))
      .sort((a, b) => a.agent.railOrder - b.agent.railOrder || a.order - b.order)
      .map((entry) => entry.agent);
  }, [agents]);
}

export function useAgentLookup(): {
  byId: (id: AgentId) => AgentCard | undefined;
  byName: (name: string) => AgentCard | undefined;
} {
  const agents = useStore((s) => s.agents);
  const byId = useCallback((id: AgentId) => agents.find((a) => a.id === id), [agents]);
  const byName = useCallback(
    (name: string) => agents.find((a) => a.name.toLowerCase() === name.trim().toLowerCase()),
    [agents],
  );
  return useMemo(() => ({ byId, byName }), [byId, byName]);
}
