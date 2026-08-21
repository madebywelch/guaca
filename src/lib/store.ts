/**
 * Application state.
 *
 * The Rust side is the source of truth for everything durable; this store is a
 * cache plus the few things that only exist while you are looking at them
 * (which channel is open, what text is mid-stream, which pulses are in flight).
 */

import { useCallback, useMemo } from "react";
import { create } from "zustand";

import { api } from "./ipc";
import { keepThought } from "./reasoning";
import type {
  Activity,
  AgentCard,
  AgentId,
  ApprovalId,
  ApprovalState,
  Envelope,
  Group,
  GroupId,
  GroupUsage,
  MessageId,
  Participant,
  Settings,
  Tokens,
  UiEvent,
} from "./types";

/** The activity feed is addressed like a channel but is not an agent. */
export const ACTIVITY_CHANNEL = "activity" as const;
export type ChannelKey = AgentId | typeof ACTIVITY_CHANNEL;

export interface StreamBuffer {
  channelId: AgentId;
  agentId: AgentId;
  text: string;
  /** Who the finished message is for. Peer-bound streams are not shown as text. */
  to: Participant;
}

/** One inter-agent message, animated down the rail. */
export interface Pulse {
  id: number;
  from: AgentId;
  to: AgentId;
  color: string;
}

/** How long the group meters look back. */
export const PULSE_WINDOW_MS = 90_000;

interface State {
  agents: AgentCard[];
  groups: Group[];
  activity: Record<AgentId, Activity>;
  /** Newest message timestamp per agent. Drives the sidebar order. */
  lastActive: Record<AgentId, number>;
  settings: Settings | null;

  /** Everything each group has spent, ever. Keyed by group id. */
  usage: Record<GroupId, Tokens | undefined>;
  /**
   * Calls in the last {@link PULSE_WINDOW_MS}, for the meters.
   *
   * Kept as raw points rather than as buckets: buckets would have to be
   * rotated on a timer whether or not anything was happening, and the one
   * thing this is for is showing that something is.
   */
  pulse: Record<GroupId, { at: number; tokens: number }[] | undefined>;

  /**
   * What each permission request came to. The request itself lives in the
   * transcript, which is immutable; this is the part that moves.
   */
  approvals: Record<ApprovalId, ApprovalState | undefined>;

  selected: ChannelKey | null;
  messages: Record<ChannelKey, Envelope[] | undefined>;
  streams: Record<MessageId, StreamBuffer | undefined>;
  /**
   * What each agent is thinking, while it is thinking it.
   *
   * Kept apart from `streams` rather than folded into the buffer, and that is
   * not tidiness: the component drawing the live bubbles subscribes to
   * `streams`, so a thought written into one would re-render and re-parse the
   * markdown of every bubble on screen for text that is not in any of them.
   * Keyed by agent because that is who is thinking; a turn writing to a peer
   * streams into the peer's channel while the operator watching it work is
   * reading its own.
   */
  reasoning: Record<AgentId, string | undefined>;
  pulses: Pulse[];

  /**
   * The message a search result asked for, until the transcript has scrolled
   * to it. Held here rather than passed as a prop because the channel it is in
   * has to be opened first, and the two happen in different renders.
   */
  focused: MessageId | null;

  /** Non-blocking surface for the last thing that went wrong. */
  banner: { tone: "error" | "info" | "ok"; text: string } | null;

  bootstrap: () => Promise<void>;
  refreshAgents: () => Promise<void>;
  refreshUsage: () => Promise<void>;
  refreshApprovals: () => Promise<void>;
  select: (key: ChannelKey) => Promise<void>;
  loadChannel: (key: ChannelKey, through?: MessageId) => Promise<void>;
  /** Opens a message's channel with the message itself in the window. */
  openMessage: (channel: AgentId, message: MessageId) => Promise<void>;
  clearFocus: () => void;
  applyEvent: (event: UiEvent) => void;
  dismissPulse: (id: number) => void;
  setBanner: (banner: State["banner"]) => void;
  setSettings: (settings: Settings) => void;
}

let pulseSeq = 0;

/** Group totals, addressed the way the UI reads them. */
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

/** Keeps a channel's messages ordered and free of duplicates. */
function insert(existing: Envelope[] | undefined, message: Envelope): Envelope[] | undefined {
  // An unloaded channel stays unloaded: appending here would produce a
  // transcript with a hole in the middle the first time it is opened.
  if (existing === undefined) return undefined;
  if (existing.some((m) => m.id === message.id)) return existing;

  const next = [...existing, message];
  next.sort((a, b) => a.createdAt - b.createdAt || (a.id < b.id ? -1 : 1));
  return next;
}

export const useStore = create<State>((set, get) => ({
  agents: [],
  groups: [],
  activity: {},
  lastActive: {},
  settings: null,
  usage: {},
  pulse: {},
  approvals: {},
  selected: null,
  messages: {},
  streams: {},
  reasoning: {},
  pulses: [],
  focused: null,
  banner: null,

  async bootstrap() {
    const [agents, groups, activity, lastActive, settings, usage, approvals] = await Promise.all([
      api.listAgents(),
      api.listGroups(),
      api.agentActivity(),
      api.agentLastActive(),
      api.getSettings(),
      api.usageSummary(),
      api.approvalStates(),
    ]);
    set({ agents, groups, activity, lastActive, settings, usage: byGroup(usage), approvals });

    const live = agents.filter((a) => a.lifecycle !== "terminated");
    const current = get().selected;
    if (!current && live.length > 0) {
      await get().select(live[0]!.id);
    }
  },

  async refreshAgents() {
    // Groups come back with the roster because an agent moving between them
    // changes both counts, and one refresh keeps the two consistent on screen.
    const [agents, groups] = await Promise.all([api.listAgents(), api.listGroups()]);
    set({ agents, groups });

    // If the open channel was just deleted, fall back rather than showing a
    // dead pane.
    const selected = get().selected;
    if (selected && selected !== ACTIVITY_CHANNEL) {
      const still = agents.find((a) => a.id === selected && a.lifecycle !== "terminated");
      if (!still) {
        const next = agents.find((a) => a.lifecycle !== "terminated");
        set({ selected: next ? next.id : ACTIVITY_CHANNEL });
        if (next) await get().loadChannel(next.id);
      }
    }
  },

  async select(key) {
    set({ selected: key, focused: null });
    await get().loadChannel(key);
  },

  async loadChannel(key, through) {
    const messages =
      key === ACTIVITY_CHANNEL
        ? await api.conversationFlow(400)
        : await api.channelMessages(key, 300, through);
    set((state) => ({ messages: { ...state.messages, [key]: messages } }));
  },

  /**
   * A search result, opened.
   *
   * The channel is read again even when it is already the open one: the window
   * on screen is the newest three hundred, and the message being asked for may
   * be older than that. `focused` is set before the read so the transcript can
   * mark the row the moment it arrives, and the channel is switched first so
   * the operator sees where they are going rather than a pause.
   */
  async openMessage(channel, message) {
    set({ selected: channel, focused: message });
    await get().loadChannel(channel, message);
  },

  clearFocus() {
    set({ focused: null });
  },

  async refreshUsage() {
    set({ usage: byGroup(await api.usageSummary()) });
  },

  /**
   * Re-reads what every request came to. Used when a decision is refused,
   * which means this side is holding a stale answer.
   */
  async refreshApprovals() {
    set({ approvals: await api.approvalStates() });
  },

  applyEvent(event) {
    switch (event.type) {
      case "agentsChanged": {
        void get().refreshAgents();
        break;
      }

      case "messageAppended": {
        const message = event.message;
        set((state) => {
          const messages = { ...state.messages };
          messages[message.channelId] = insert(messages[message.channelId], message);
          // The flow board covers the whole conversation, so everything except
          // an agent's private activity record belongs on it.
          if (message.to.kind !== "system") {
            messages[ACTIVITY_CHANNEL] = insert(messages[ACTIVITY_CHANNEL], message);
          }

          // Draw the pulse from the event, not from a channel read, so it
          // fires whether or not either channel is open. Bound to locals
          // because narrowing on a property does not survive into a callback.
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

          // Both ends count as active: a message an agent sent lives in the
          // recipient's channel, so tracking the channel alone would leave the
          // sender looking idle and sink it down the rail.
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
          // Whatever this agent was thinking belonged to the attempt this one
          // replaces. A failed call reopens under a new id, and its half-formed
          // last thought is not what the retry is doing.
          const reasoning = { ...state.reasoning };
          delete reasoning[event.agentId];
          return {
            reasoning,
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
          if (!current) return state;
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
          // The stream is what says whose thought this is, and whether anything
          // is still waiting for it. A delta for a placeholder that has already
          // gone is dropped, exactly as its text would be.
          const stream = state.streams[event.messageId];
          if (!stream) return state;
          return {
            reasoning: {
              ...state.reasoning,
              [stream.agentId]: keepThought(state.reasoning[stream.agentId], event.text),
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

          // The turn is over, so the thinking goes with it. This is the whole
          // of what makes it ephemeral: nothing else ever clears it.
          const reasoning = { ...state.reasoning };
          delete reasoning[ending.agentId];
          return { streams, reasoning };
        });
        break;
      }

      case "activityChanged": {
        set((state) => ({
          activity: { ...state.activity, [event.agentId]: event.activity },
        }));
        break;
      }

      case "channelsCleared": {
        // Dropping the cache is not enough on its own: the channel on screen
        // has to be read again, or it keeps showing what it already had until
        // the operator clicks away and back, which is exactly what they had to
        // do before this event existed.
        const emptied = new Set<ChannelKey>(event.agents);
        set((state) => {
          const messages = { ...state.messages };
          for (const key of Object.keys(messages) as ChannelKey[]) {
            // The activity feed draws from every channel, so it is stale too.
            if (emptied.has(key) || key === ACTIVITY_CHANNEL) delete messages[key];
          }
          return { messages };
        });

        const open = get().selected;
        if (open && (emptied.has(open) || open === ACTIVITY_CHANNEL)) {
          void get().loadChannel(open);
        }
        // The meters are counting rows that no longer exist.
        void get().refreshUsage();
        break;
      }

      case "tokensUsed": {
        // Applied here rather than refetched: the whole point is a number that
        // moves while an agent is still working. `runSettled` reconciles.
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
          return {
            usage: {
              ...state.usage,
              [event.groupId]: {
                prompt: held.prompt + event.prompt,
                completion: held.completion + event.completion,
                // Null stays null: a provider that prices nothing has not
                // made this run free, and adding it up as zero would say so.
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

      case "runSettled":
        // The live totals are additions to a number this corrects. Cheap: one
        // grouped sum over a local table, and only when a run has gone quiet.
        void get().refreshUsage();
        break;

      case "approvalRequested": {
        set((state) => ({
          approvals: { ...state.approvals, [event.approvalId]: "pending" },
        }));
        break;
      }

      case "approvalSettled": {
        set((state) => ({
          approvals: { ...state.approvals, [event.approvalId]: event.state },
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

  setSettings(settings) {
    set({ settings });
  },
}));

/**
 * Agents shown in the rail, most recently active first.
 *
 * Agents that have never spoken keep their creation order at the bottom, so a
 * fresh workspace reads in the order you built it and a busy one floats whoever
 * just spoke to the top.
 *
 * Memoized, and that is load-bearing rather than an optimization. `filter`
 * allocates a new array on every render, so an unmemoized result is a fresh
 * reference each time. Put that in a dependency list next to a `setState` and
 * you get effect -> render -> new reference -> effect, which React aborts by
 * unmounting the whole tree. The window paints its background and nothing else.
 */
export function useLiveAgents(): AgentCard[] {
  const agents = useStore((s) => s.agents);
  const lastActive = useStore((s) => s.lastActive);

  return useMemo(() => {
    return agents
      .filter((a) => a.lifecycle !== "terminated")
      .map((agent, order) => ({ agent, order, at: lastActive[agent.id] ?? 0 }))
      .sort((a, b) => b.at - a.at || a.order - b.order)
      .map((entry) => entry.agent);
  }, [agents, lastActive]);
}

/**
 * Resolves agents for rendering history. Deleted agents are included: they
 * still appear in transcripts, and a message from a nameless id is unreadable.
 */
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
