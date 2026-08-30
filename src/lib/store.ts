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
import { loadPrefs, type Prefs, savePrefs } from "./prefs";
import { type DropTarget, landsBefore, railOrder } from "./rail";

/**
 * How much of a running coding job's work is kept on screen.
 *
 * Enough to see what it has been doing, not enough to be a transcript. The
 * transcript is the message the job delivers when it ends.
 */
const CODING_TAIL = 40;

/** A desktop's answer, which is every capability there is. */
const EVERYTHING: Capabilities = {
  localDirectories: true,
  loopbackEndpoints: true,
  claudeProvider: true,
  claudeCodeHarness: true,
  localFiles: true,
};

import { keepThought } from "./reasoning";
import type { LiveCall } from "./trail";
import type {
  Activity,
  AgentCard,
  AgentId,
  Approval,
  ApprovalId,
  ApprovalState,
  Capabilities,
  CodingLine,
  Decision,
  Envelope,
  Escalation,
  EscalationId,
  Group,
  GroupId,
  GroupUsage,
  MessageId,
  Participant,
  RepoStatus,
  Repository,
  RepositoryId,
  RoutineId,
  RunId,
  Settings,
  Tokens,
  UiEvent,
} from "./types";
import { errorMessage } from "./types";

/**
 * A channel is an agent, and nothing else is one.
 *
 * The flow board used to be addressed as a channel here, which made every
 * function that took a channel take a value that was not an agent and cost
 * seven of them a branch. It is analysis rather than a place you talk, so it
 * moved into the group editor and reads its own history: `GroupActivity`.
 */
export type ChannelKey = AgentId;

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

/** Where a move leaves an agent. */
export interface Placement {
  groupId: GroupId;
  /** The row it lands in front of. `null` is the end of the group. */
  before: AgentId | null;
  /** Absent leaves the section it is in alone. */
  pinned?: boolean;
}

interface State {
  agents: AgentCard[];
  groups: Group[];
  /**
   * Every repository in the workspace, filtered per crew where it is drawn.
   *
   * Beside the roster rather than fetched by whoever draws it, for the reason
   * groups are: an agent given a repository changes what two panels say, and
   * one refresh keeps them consistent.
   */
  repositories: Repository[];
  /**
   * Agents with a coding job running, and the repository each is working in.
   *
   * Keyed by agent rather than by repository, because a repository that gives
   * each agent a worktree of its own can have two jobs running in it and would
   * name neither. An agent works in at most one repository and holds at most
   * one work tree in it, so it names exactly one. It is also the direction both
   * readers wanted: `CodingPanel` had to search the map by agent, and the rail
   * only asks whether anything at all is building in a repository.
   *
   * In memory and event-driven, like the job itself. It does not survive a
   * restart, which is correct: neither does the job.
   */
  building: Record<AgentId, RepositoryId>;
  /**
   * What each running job is doing, newest last, by the agent that started it.
   *
   * Bounded and ephemeral: dropped when the job ends, because the record of
   * what a job did is the message it delivers. Keyed by agent because that is
   * whose channel draws it.
   */
  coding: Record<AgentId, CodingLine[]>;
  /**
   * What each linked repository is doing, by id.
   *
   * Separate from the repositories themselves because it has a different
   * lifetime: the row changes when the operator links or renames one, and this
   * changes when they commit, in a terminal Guaca never sees. Absent for a
   * repository whose directory could not be read.
   */
  repoStatus: Record<RepositoryId, RepoStatus>;
  activity: Record<AgentId, Activity>;
  /** Newest message timestamp per agent. Drives the sidebar order. */
  lastActive: Record<AgentId, number>;
  settings: Settings | null;
  /**
   * What this workspace can do, read once when the window opens.
   *
   * Everything on a desktop, and the desktop's answer is also what is assumed
   * until the read lands: a panel drawn before it can offer nothing a desktop
   * would not, and nothing draws before `ready` anyway. On a server the five
   * flags are what stand between an operator and a control that fails only
   * after they have filled it in.
   */
  capabilities: Capabilities;
  /** Local preferences. See `lib/prefs`: the runtime never reads these. */
  prefs: Prefs;
  /**
   * The conversation each agent is currently part of, so it can be stopped.
   *
   * Learned from the placeholder that opens in the agent's channel, which is
   * the runtime's own statement that this agent is working on that run, and
   * dropped when the run settles. Not read from `sendMessage`'s return value:
   * that only knows about conversations the operator started, and a routine or
   * a peer's request is exactly as worth stopping.
   *
   * Keyed by agent rather than by run because that is the question the button
   * asks: the operator is looking at one channel and wants what is happening in
   * it to stop.
   */
  activeRun: Record<AgentId, RunId | undefined>;

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

  /**
   * Every request still waiting on the operator, oldest first, whole.
   *
   * The queue the desk draws. Held as what the runtime last said rather than
   * accumulated from events, which is the same discipline the menu bar's
   * presence is built on and for the same reason: a list added to on
   * `approvalRequested` and removed from on `approvalSettled` is one dropped
   * event away from offering a decision that reaches nobody, and the operator
   * has no way to tell that card from a live one. Both events invalidate this
   * and the answer comes back from the runtime.
   *
   * Whole rather than by id because the desk exists to be answered from: an
   * entry that carried only an id would send the operator to find the channel,
   * which is the walk the desk is for.
   */
  pending: Approval[];

  /**
   * Every escalation still open, oldest first.
   *
   * The other half of the desk's queue, held the same way and for the same
   * reason. Beside `pending` rather than merged into it because the two are
   * answered differently: one takes a verdict or a value and has ten minutes to
   * take it in, and this one is cleared when the operator has dealt with it and
   * waits as long as it takes.
   */
  stuck: Escalation[];

  selected: ChannelKey | null;
  /**
   * The group the rail is looking inside, or `null` for all of them.
   *
   * Here rather than in the sidebar because it and the open channel have to
   * agree, and both are here. One rule holds them together: the rail draws the
   * row of the channel the pane is showing. `select` repairs it by following the
   * agent into its crew, and `focusGroup` repairs it from the other end, by
   * letting go of a channel the crew being opened does not contain.
   */
  railGroup: GroupId | null;
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
  /**
   * What each agent's turn has reached for, while it is reaching.
   *
   * The same lifetime as `reasoning`, from the same mechanism: the runtime
   * addresses both to the placeholder, so both are filed under the agent that
   * opened it and both go when it ends. What a turn did is the message that
   * lands at the end of it, and the transcript draws these chips again from
   * that; this is the only account of the same work while it is still under
   * way, which for a turn that spends ten minutes on tool calls is the whole of
   * what there is to watch.
   */
  trail: Record<AgentId, LiveCall[] | undefined>;
  /**
   * When each agent's last reply landed.
   *
   * A stamp rather than a flag, so the one thing that reads it can decide for
   * itself how long a finished turn is worth looking pleased about without a
   * timer here putting it back. Never cleared: the value is what it says, and
   * an agent that finished an hour ago is not a different fact from an agent
   * that has never run.
   */
  finishedAt: Record<AgentId, number | undefined>;
  pulses: Pulse[];

  /**
   * The message a search result asked for, until the transcript has scrolled
   * to it. Held here rather than passed as a prop because the channel it is in
   * has to be opened first, and the two happen in different renders.
   */
  focused: MessageId | null;

  /**
   * The routine a transcript row asked the panel to open, until it has.
   *
   * Held here for the same reason as `focused`: the two ends are in different
   * columns of the window. A fired routine is drawn in the channel and read in
   * the panel beside it, and threading a callback from the app root through
   * every message to get there would put a prop on every row for the sake of
   * one kind of chip.
   */
  openingRoutine: RoutineId | null;

  /**
   * How many times each agent's schedule has changed since the window opened.
   *
   * A counter rather than the routines themselves: the panel reads them from
   * Rust when it draws, and holding a second copy here would be a cache to keep
   * in step with the one the component already has. What the component cannot
   * work out for itself is *when* to read again, and an agent that sets a
   * routine mid-turn is exactly when: the list was drawn before the routine
   * existed, and closing the panel and opening it again was the only way to see
   * it.
   */
  routineVersion: Record<AgentId, number | undefined>;

  /**
   * The same counter for each agent's memory, and it is a second one rather
   * than a share of the first. A schedule and a memory move on different
   * occasions, and a panel that read the whole file back every time a routine
   * came due would throw away an edit the operator was in the middle of
   * writing.
   */
  memoryVersion: Record<AgentId, number | undefined>;
  /** Its own counter, because notes move far more often than memory does. */
  workingNotesVersion: Record<AgentId, number | undefined>;

  /** Non-blocking surface for the last thing that went wrong. */
  banner: { tone: "error" | "info" | "ok"; text: string } | null;

  bootstrap: () => Promise<void>;
  refreshAgents: () => Promise<void>;
  /**
   * Asks git, and `gh`, what the linked repositories are doing.
   *
   * Polled rather than pushed. Nothing that changes a branch or opens a pull
   * request goes through Guaca, so there is no event to listen for and the
   * only honest options are asking again or being wrong.
   */
  refreshRepoStatuses: () => Promise<void>;
  refreshUsage: () => Promise<void>;
  refreshApprovals: () => Promise<void>;
  /**
   * Re-reads the open escalations.
   *
   * Its own call rather than a third read inside `refreshApprovals`: the two
   * queues change on different events, and folding them together would make
   * every answered permission re-read a list that cannot have moved.
   */
  refreshEscalations: () => Promise<void>;
  /** Takes one off the desk. Nothing is waiting on it, so nothing resumes. */
  clearEscalation: (id: EscalationId) => Promise<void>;
  select: (key: ChannelKey) => Promise<void>;
  /**
   * Looks inside one group, or back out at all of them. Closes the open channel
   * if the crew being opened is not the one it belongs to.
   */
  focusGroup: (id: GroupId | null) => Promise<void>;
  /**
   * Puts an agent somewhere: which group, in front of which row, and whether it
   * is pinned. Absent `pinned` leaves the section it is in alone.
   */
  moveAgent: (id: AgentId, at: Placement) => Promise<void>;
  /** One drop, resolved against the rules that drew the rail. */
  dropAgent: (id: AgentId, target: DropTarget) => Promise<void>;
  loadChannel: (key: ChannelKey, through?: MessageId) => Promise<void>;
  /** Opens a message's channel with the message itself in the window. */
  openMessage: (channel: AgentId, message: MessageId) => Promise<void>;
  clearFocus: () => void;
  /** Asks the panel beside the transcript to open one routine. */
  openRoutine: (id: RoutineId) => void;
  routineOpened: () => void;
  /**
   * Answers one request, from wherever it was seen.
   *
   * In the store rather than in the card because there are two cards: the one
   * in the transcript and the one on the desk, and the same request is live on
   * both at once. A refusal here is the runtime saying this was already
   * answered, or that it lapsed while the operator was reading it, and the
   * runtime's copy is the truth: it is taken rather than argued with, and both
   * readings of it are corrected together.
   */
  decideApproval: (id: ApprovalId, decision: Decision) => Promise<void>;
  /**
   * Answers a question with what the operator picked or wrote.
   *
   * Beside the verdict rather than folded into it, because a question is
   * settled with a value and a permission with one of three tokens. Everything
   * after the call is the same, which is why both go through one refresh.
   */
  answerQuestion: (id: ApprovalId, answer: string) => Promise<void>;
  /**
   * One request answered, however it was answered.
   *
   * Both ways in do the same two things afterward, and this is the one place
   * that knows what they are: a refusal is the runtime saying this was already
   * settled or lapsed while it was being read, and its copy is the truth, so it
   * is taken rather than argued with and both readings of it are corrected
   * together. Private in spirit; on the store because the two callers are.
   */
  settle: (answer: () => Promise<unknown>) => Promise<void>;
  applyEvent: (event: UiEvent) => void;
  dismissPulse: (id: number) => void;
  setBanner: (banner: State["banner"]) => void;
  setSettings: (settings: Settings) => void;
  /**
   * Merges a change into the local preferences and writes them back.
   *
   * The write is here rather than in an effect beside the reader, so there is
   * one place a preference is persisted and no component has to remember to.
   */
  setPrefs: (patch: Partial<Prefs>) => void;
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

/**
 * Which crew the rail is inside once a channel has been opened.
 *
 * One invariant, from this end: the rail has to be drawing the row of whatever
 * the pane is showing. A search hit can land on a member of any crew, and a
 * rail still showing the one you were in has the open channel nowhere on it.
 *
 * It follows the agent rather than dropping out to the overview, which is what
 * it used to do. Dropping out was the only honest answer while the crews lived
 * in a strip inside the rail: going to the overview was the one state where
 * every row was drawable, and which crew you had ended up in was a heading you
 * had to read. The crews now have a column of their own that is on screen
 * whichever one you are in, so following is both possible and less: one lit
 * circle moves, instead of the whole rail changing shape.
 *
 * The overview stays the overview. It draws everybody, so a channel opened from
 * it is already on screen and there is nothing to repair.
 */
function keptFocus(state: State, key: ChannelKey): GroupId | null {
  if (state.railGroup === null) return state.railGroup;
  const agent = state.agents.find((a) => a.id === key);
  return agent ? agent.groupId : state.railGroup;
}

/**
 * Whether the open channel survives the rail going inside a group.
 *
 * The same invariant as `keptFocus` read from the other end: the rail has to be
 * able to draw the channel that is open. Going back out to the overview keeps
 * whatever was open, because the overview draws everybody.
 *
 * This end still lets go rather than following, and the asymmetry is the point.
 * `select` is the operator naming an agent, so taking them to that agent's crew
 * is what they asked for. `focusGroup` is the operator naming a crew, and
 * following the channel out of it would undo the click.
 */
function keptChannel(state: State, group: GroupId | null): boolean {
  const key = state.selected;
  if (group === null || key === null) return true;
  const agent = state.agents.find((a) => a.id === key);
  return agent === undefined || agent.groupId === group;
}

export const useStore = create<State>((set, get) => ({
  agents: [],
  groups: [],
  repositories: [],
  building: {},
  coding: {},
  repoStatus: {},
  activity: {},
  lastActive: {},
  settings: null,
  capabilities: EVERYTHING,
  prefs: loadPrefs(),
  activeRun: {},
  usage: {},
  pulse: {},
  approvals: {},
  pending: [],
  stuck: [],
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
  memoryVersion: {},
  workingNotesVersion: {},
  banner: null,

  async bootstrap() {
    const [
      agents,
      groups,
      repositories,
      activity,
      lastActive,
      settings,
      capabilities,
      usage,
      approvals,
      pending,
      stuck,
    ] = await Promise.all([
      api.listAgents(),
      api.listGroups(),
      api.listRepositories(),
      api.agentActivity(),
      api.agentLastActive(),
      api.getSettings(),
      api.capabilities(),
      api.usageSummary(),
      api.approvalStates(),
      // A turn parked before the window was opened is still parked. The desk
      // has to be right on the first paint, or the operator's first read of
      // it says nobody is waiting.
      api.pendingApprovals(),
      // And an agent that gave up on a Friday is still stuck on Monday. This
      // one has no window at all, so first paint is the only thing that decides
      // whether the operator ever sees it.
      api.openEscalations(),
    ]);
    set({
      agents,
      groups,
      repositories,
      activity,
      lastActive,
      settings,
      capabilities,
      usage: byGroup(usage),
      approvals,
      pending,
      stuck,
    });

    const live = agents.filter((a) => a.lifecycle !== "terminated");
    const current = get().selected;
    if (!current && live.length > 0) {
      await get().select(live[0]!.id);
    }
  },

  async refreshRepoStatuses() {
    try {
      set({ repoStatus: await api.repositoryStatuses() });
    } catch {
      // Left as it was rather than cleared. A failed poll is usually a
      // directory that is momentarily busy, and blanking every branch name for
      // one bad read makes the rail flicker on a timer.
    }
  },

  async refreshAgents() {
    // Groups come back with the roster because an agent moving between them
    // changes both counts, and one refresh keeps the two consistent on screen.
    const [agents, groups, repositories] = await Promise.all([
      api.listAgents(),
      api.listGroups(),
      api.listRepositories(),
    ]);
    // A group the rail was looking inside can be deleted from the group editor,
    // and a focus on one that is gone draws an empty rail with no way out of it.
    set((state) => ({
      agents,
      groups,
      repositories,
      railGroup: groups.some((g) => g.id === state.railGroup) ? state.railGroup : null,
    }));

    // If the open channel was just deleted, fall back rather than showing a
    // dead pane.
    const selected = get().selected;
    if (selected) {
      const still = agents.find((a) => a.id === selected && a.lifecycle !== "terminated");
      if (!still) {
        const next = agents.find((a) => a.lifecycle !== "terminated");
        set({ selected: next ? next.id : null });
        if (next) await get().loadChannel(next.id);
      }
    }
  },

  async select(key) {
    // A pending routine request goes with the channel it was asked from, for
    // the same reason `focused` does: it is about a row on the screen the
    // operator is leaving, and honoring it later would open something they
    // asked for somewhere else.
    set((state) => ({
      selected: key,
      focused: null,
      openingRoutine: null,
      railGroup: keptFocus(state, key),
    }));
    await get().loadChannel(key);
  },

  /**
   * A crew, opened.
   *
   * A channel from the crew you came from does not stay open behind it. The rail
   * would not be drawing its row, so there is nothing on screen that says which
   * crew the pane belongs to, and two crews can hold two agents with the same
   * name and the same face: one left open from the group you just left reads as
   * a member of the group you are looking at, working while nobody here is. The
   * agent goes on working. It is the reading of it that was wrong.
   *
   * What it falls back to is nothing, rather than the first row of the crew
   * being entered. Opening a channel is the operator naming somebody, and a
   * crew that picked one for them would put an agent's history on screen as a
   * side effect of a click that was about the crew.
   */
  async focusGroup(id) {
    const closing = !keptChannel(get(), id);
    set({ railGroup: id, ...(closing ? { selected: null, focused: null } : {}) });
  },

  /**
   * The whole of a move, applied.
   *
   * Read back rather than patched locally. The runtime renumbers every live row
   * to close the gap the agent left, so the one card that came back is not the
   * change: guessing the rest here would leave the rail drawing an arrangement
   * the database does not have until the next unrelated refresh.
   *
   * Pinning first, and only when it differs. It is a separate command because
   * it is a separate fact, and doing it second would mean a refresh that draws
   * the row in its new place and its old section.
   */
  async moveAgent(id, at) {
    const current = get().agents.find((a) => a.id === id);
    if (at.pinned !== undefined && current && current.pinned !== at.pinned) {
      await api.setAgentPinned(id, at.pinned);
    }
    await api.moveAgent(id, at.groupId, at.before);
    await get().refreshAgents();
  },

  /**
   * One drop, resolved.
   *
   * Which section the target row belongs to is worked out here rather than
   * handed in, because the same rules that drew the rail have to decide where
   * the row lands and a component recomputing them would be a second place for
   * them to drift.
   *
   * A pin is the head of a crew, so the row landed on says both things at once:
   * which crew, and whether the place aimed at is the pinned band above it or
   * the rest below. Dropping onto a pinned row pins the dragged agent, dropping
   * below the band unpins it, and either can also change the crew.
   *
   * Dropping on a group is the one gesture that says nothing about the band, so
   * it says nothing: the pin is the operator's standing instruction about that
   * agent, and moving somebody between crews is not a decision to drop it.
   */
  async dropAgent(id, target) {
    const state = get();
    const dragged = state.agents.find((a) => a.id === id);
    if (!dragged) return;

    const live = state.agents.filter((a) => a.lifecycle !== "terminated");

    if (target.kind === "group") {
      await get().moveAgent(id, { groupId: target.id, before: null });
      return;
    }

    // A move, like dropping on a crew, but inside the crew: an agent works in
    // at most one repository, so this replaces whatever it was in rather than
    // adding to it. Dropping it back where it already is changes nothing, which
    // is what makes an accidental drag free.
    if (target.kind === "repository") {
      const repository = state.repositories.find((r) => r.id === target.id);
      if (!repository || repository.id === dragged.repositoryId) return;
      // The store refuses this anyway. Refused here too so a drag across a
      // crew boundary is a gesture that does nothing rather than one that
      // raises an error the rail has nowhere to put.
      if (repository.groupId !== dragged.groupId) return;
      await api.setAgentRepository(id, target.id);
      await get().refreshAgents();
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

  async loadChannel(key, through) {
    const messages = await api.channelMessages(key, 300, through);
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

  async refreshUsage() {
    set({ usage: byGroup(await api.usageSummary()) });
  },

  /**
   * Re-reads both halves of what is being asked: what every request came to,
   * and which are still waiting.
   *
   * One call for the two, because they are two views of one table and a refresh
   * that took only one leaves the transcript's buttons and the desk's queue
   * disagreeing about the same request.
   */
  async refreshApprovals() {
    const [approvals, pending] = await Promise.all([api.approvalStates(), api.pendingApprovals()]);
    set({ approvals, pending });
  },

  async refreshEscalations() {
    set({ stuck: await api.openEscalations() });
  },

  async clearEscalation(id) {
    try {
      await api.clearEscalation(id);
    } catch (error) {
      get().setBanner({ tone: "error", text: errorMessage(error) });
    }
    // Read back either way, for the reason `settle` does: a clear that failed
    // must not leave the desk missing a row the store still has, and one that
    // worked is confirmed rather than assumed.
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
      get().setBanner({ tone: "error", text: errorMessage(error) });
    }
    // Read back whether it was taken or refused. A refusal means this side was
    // holding a stale answer; a success is confirmed by the same read rather
    // than assumed, so one path corrects the queue and there is no state where
    // the desk has dropped a card the runtime still has.
    await get().refreshApprovals();
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
          // last thought is not what the retry is doing. The trail goes with
          // it, for the plainer reason that this is where a turn begins.
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

      case "toolStarted": {
        set((state) => {
          // The stream says whose call this is, exactly as it does for a
          // thought, and a call for a placeholder that has gone is dropped for
          // the same reason: there is nobody left waiting on it.
          const stream = state.streams[event.messageId];
          if (!stream) return state;
          const call: LiveCall = {
            callId: event.callId,
            name: event.name,
            arguments: event.arguments,
            done: null,
            // Read here rather than sent, because what this is used for is how
            // long the operator has been waiting, and the operator's clock is
            // the one they are waiting by.
            startedAt: Date.now(),
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

          // The turn is over, so the thinking goes with it, and so does the
          // record of what it reached for. This is the whole of what makes both
          // ephemeral: nothing else ever clears them.
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
          // An agent that has gone quiet is not working on anything, so the run
          // it was working on stops being the one to stop. Left behind, the
          // entry would still be here when the agent was next handed work, and
          // the Stop button would name the conversation before this one.
          // Absent is the right kind of wrong: no button beats the wrong one.
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
            if (emptied.has(key)) delete messages[key];
          }
          return { messages };
        });

        const open = get().selected;
        if (open && emptied.has(open)) {
          void get().loadChannel(open);
        }
        // The panels beside the transcript hold the two stores a reset also
        // takes, and neither is read again on its own. Left alone, the inspector
        // draws a memory and a list of working notes that no longer exist,
        // beside the empty channel that says they should not.
        set((state) => {
          const memory = { ...state.memoryVersion };
          const working = { ...state.workingNotesVersion };
          for (const agent of event.agents) {
            memory[agent] = (memory[agent] ?? 0) + 1;
            working[agent] = (working[agent] ?? 0) + 1;
          }
          return { memoryVersion: memory, workingNotesVersion: working };
        });
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

      case "runSettled": {
        // Every agent that was working on this one has stopped, whether it
        // finished, was refused or was stopped. Cleared by run rather than by
        // agent because a cascade leaves several entries pointing at it.
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
        // The live totals are additions to a number this corrects. Cheap: one
        // grouped sum over a local table, and only when a run has gone quiet.
        void get().refreshUsage();
        break;
      }

      // Both of these patch the state map and then re-read the queue, which
      // looks like one thing done twice and is not. The map is what the buttons
      // on a request already drawn in a transcript are keyed on, and patching it
      // is what makes them go live or go dead in the same frame as the event.
      // The queue is the wording of every request still waiting, which no event
      // carries: `approvalRequested` says an id and an agent, deliberately, so
      // the read is where the desk's card comes from.
      case "approvalRequested": {
        set((state) => ({
          approvals: { ...state.approvals, [event.approvalId]: "pending" },
        }));
        void get().refreshApprovals();
        break;
      }

      case "approvalSettled": {
        set((state) => ({
          approvals: { ...state.approvals, [event.approvalId]: event.state },
        }));
        void get().refreshApprovals();
        break;
      }

      // Nothing to patch first, unlike the two above: an escalation has no
      // state a card is keyed on, and the wording is the whole row. Both events
      // invalidate the queue and the read is where the desk's card comes from.
      case "escalationRaised":
      case "escalationCleared": {
        void get().refreshEscalations();
        break;
      }

      // Onto the banner rather than into a channel. An expired credential on
      // the operator's own machine is theirs to fix, and it stopped every
      // coding job in the workspace while every agent reported that nothing
      // needed doing.
      case "codingJobStarted": {
        set((state) => ({
          building: { ...state.building, [event.agentId]: event.repositoryId },
        }));
        break;
      }

      case "codingProgress": {
        set((state) => {
          // Bounded. A long job runs hundreds of tools, and an unbounded tail
          // is a growing array re-rendered on every line of it.
          const held = [
            ...(state.coding[event.agentId] ?? []),
            {
              tool: event.tool,
              detail: event.detail,
            },
          ].slice(-CODING_TAIL);
          return { coding: { ...state.coding, [event.agentId]: held } };
        });
        break;
      }

      case "codingJobFinished": {
        set((state) => {
          const { [event.agentId]: _gone, ...rest } = state.building;
          const { [event.agentId]: _done, ...others } = state.coding;
          return { building: rest, coding: others };
        });
        break;
      }

      case "codingJobFailed": {
        // The harness is named, because the way out of the commonest failure
        // here is the other one: a spent plan is not something the operator can
        // fix from inside this app, and a banner that does not say what was
        // running leaves them guessing which sign-in to go and look at.
        get().setBanner({
          tone: "error",
          text: `A coding job in ${event.repository} could not run on ${event.harness}: ${event.reason}`,
        });
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

  setPrefs(patch) {
    set((state) => {
      const prefs = { ...state.prefs, ...patch };
      savePrefs(prefs);
      return { prefs };
    });
  },
}));

/**
 * Every agent still in the workspace, in the order the operator arranged them.
 *
 * The arrangement and not the drawn order: this is one flat list across every
 * group, and where a row is lifted for working is a question about one section
 * of the rail. `lib/rail.ts` answers that, and the sidebar asks it per section.
 * A caller that just wants the roster, like the composer's mention list, gets
 * the arrangement, which is the order the operator would look for a name in.
 *
 * Memoized, and that is load-bearing rather than an optimization. `filter`
 * allocates a new array on every render, so an unmemoized result is a fresh
 * reference each time. Put that in a dependency list next to a `setState` and
 * you get effect -> render -> new reference -> effect, which React aborts by
 * unmounting the whole tree. The window paints its background and nothing else.
 */
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
