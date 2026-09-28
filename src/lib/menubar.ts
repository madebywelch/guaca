/**
 * The menu bar, as the webview sees it: what the window hands the icon, and
 * what the panel under the icon draws.
 *
 * The icon is drawn by Rust from a presence the window reports, because the
 * tray process holds no workspace of its own. `presenceOf` is that report, one
 * projection of the store, and `ipc.contract.test.ts` holds its fields equal to
 * the struct that reads it.
 *
 * The panel is a second client of the same host with its own store, so it
 * needs no report: everything below `samePresence` is what it decides about
 * that store. No DOM here, so the suite beside this file is where those
 * decisions are argued.
 */

import { needsAnswer } from "./decisions";
import type { State, StreamBuffer } from "./store";
import type { Activity, AgentCard, AgentId, Envelope, Presence } from "./types";

/** The icon's view of the store, in the shape `menubar::Presence` reads. */
export function presenceOf(state: State): Presence {
  const roster: Presence["roster"] = {};
  for (const agent of state.agents) {
    if (agent.lifecycle === "terminated") continue;
    roster[agent.id] = { name: agent.name, crew: agent.groupId };
  }
  return {
    roster,
    crews: state.groups.map((group) => ({ id: group.id, name: group.name })),
    activity: state.activity,
    waiting: state.pending,
    stuck: state.stuck,
    decisions: state.decisions.filter(needsAnswer),
    session: state.sessionSpend,
    running: Object.values(state.activeRun).filter((run) => run !== undefined).length,
  };
}

/**
 * How long a burst of changes becomes one report.
 *
 * A streaming turn changes the store on every token, and the icon only has a
 * glyph to change. Rust coalesces on its side too; this keeps the bridge quiet.
 */
export const FEED_COALESCE_MS = 200;

/**
 * Whether two presences would draw the same icon.
 *
 * Cheap enough to run on every store change and honest enough to skip the
 * report when nothing the icon draws has moved. A structural compare rather
 * than identity, because the store rebuilds objects it did not change.
 */
export function samePresence(a: Presence, b: Presence): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Whether an agent is doing something now, as opposed to having done it. */
function busy(state: Activity | undefined, building: string | undefined): boolean {
  if (building !== undefined) return true;
  return state?.state === "thinking" || state?.state === "queued";
}

/**
 * The panel's list of agents, top to bottom.
 *
 * Whoever is working first, then whoever spoke last, then the paused. Not the
 * rail's order, which is an arrangement the operator made by hand and is right
 * for a column they navigate. The panel is opened to answer "what is happening",
 * and an agent arranged at the top that has not said anything since Tuesday is
 * the wrong first answer to it. A parked turn counts as working: it is waiting
 * on a person, which the card above the list is for, and it has not finished.
 *
 * Alphabetical at the end so two agents that have never spoken hold still.
 */
export function panelOrder(
  agents: AgentCard[],
  state: {
    activity: Record<AgentId, Activity>;
    building: Record<AgentId, string>;
    lastActive: Record<AgentId, number>;
  },
): AgentCard[] {
  const rank = (agent: AgentCard) => {
    const now = state.activity[agent.id];
    if (busy(now, state.building[agent.id]) || now?.state === "awaitingApproval") return 0;
    return agent.lifecycle === "paused" || now?.state === "paused" ? 2 : 1;
  };
  return agents
    .filter((agent) => agent.lifecycle !== "terminated")
    .map((agent) => ({ agent, rank: rank(agent), at: state.lastActive[agent.id] ?? 0 }))
    .sort(
      (a, b) =>
        a.rank - b.rank || b.at - a.at || a.agent.name.localeCompare(b.agent.name, undefined),
    )
    .map((entry) => entry.agent);
}

/**
 * The one line in the panel's header.
 *
 * The same question the icon's glyph answers, in words: is anything over there
 * mine, and if not, is anything happening. Only one of the two, because the
 * card list below says the rest and a header that said everything would be
 * read once and then skipped.
 */
export function headline(waiting: number, working: number): string {
  if (waiting > 0) return `${waiting} waiting on you`;
  if (working > 0) return `${working} working`;
  return "Nothing running";
}

/** How many agents the headline counts as working. */
export function workingCount(
  agents: AgentCard[],
  activity: Record<AgentId, Activity>,
  building: Record<AgentId, string>,
): number {
  return agents.filter((agent) => busy(activity[agent.id], building[agent.id])).length;
}

/** The words of a message, whatever else it carried. Empty when it said nothing. */
export function said(message: Envelope): string {
  return message.parts
    .flatMap((part) => (part.type === "text" ? [part.text.trim()] : []))
    .filter(Boolean)
    .join("\n\n");
}

/**
 * The last few things the operator and one agent said to each other.
 *
 * Only the two of them, and only words. The channel also holds what peers
 * filed there, what tools returned and what routines fired, and every one of
 * those is worth the whole transcript and none of them fits a panel the width
 * of a phone: the panel is for picking the conversation back up, and the
 * window is one click away for reading the rest of it.
 */
export function exchange(
  messages: Envelope[] | undefined,
  agent: AgentId,
  count: number,
): Envelope[] {
  const between = (message: Envelope) =>
    (message.from.kind === "human" && message.to.kind === "agent" && message.to.id === agent) ||
    (message.from.kind === "agent" && message.from.id === agent && message.to.kind === "human");
  return (messages ?? [])
    .filter((message) => between(message) && said(message) !== "")
    .slice(-count);
}

/**
 * What an agent is writing to the operator right now, if anything.
 *
 * Only a reply on its way to the operator. A stream to a peer is work the
 * transcript folds away, and drawing it here would be the panel showing the
 * crew's internal traffic as though it were addressed to the person reading.
 */
export function liveReply(
  streams: Record<string, StreamBuffer | undefined>,
  agent: AgentId,
): string | null {
  for (const stream of Object.values(streams)) {
    if (!stream || stream.agentId !== agent || stream.channelId !== agent) continue;
    if (stream.to.kind === "human" && stream.text.trim() !== "") return stream.text;
  }
  return null;
}
