import { needsAnswer } from "./decisions";
import { finished } from "./outcomes";
import { questions } from "./questions";
import type { State, StreamBuffer } from "./store";
import type { Activity, AgentCard, AgentId, Envelope, Presence } from "./types";

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
    finished: finished(state.outcomes).length,
  };
}

export const FEED_COALESCE_MS = 200;

export function samePresence(a: Presence, b: Presence): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

export function presenceReporter<T>(send: (value: T) => Promise<void>, equal = Object.is) {
  let acknowledged: T | undefined;
  let hasAcknowledged = false;
  let queue = Promise.resolve();
  return (value: T): Promise<void> => {
    queue = queue.then(async () => {
      if (hasAcknowledged && equal(value, acknowledged as T)) return;
      try {
        await send(value);
        acknowledged = value;
        hasAcknowledged = true;
      } catch {
        return;
      }
    });
    return queue;
  };
}

function busy(state: Activity | undefined): boolean {
  return state?.state === "thinking" || state?.state === "queued";
}

export function panelOrder(
  agents: AgentCard[],
  state: {
    activity: Record<AgentId, Activity>;
    lastActive: Record<AgentId, number>;
  },
): AgentCard[] {
  const rank = (agent: AgentCard) => {
    const now = state.activity[agent.id];
    if (busy(now) || now?.state === "awaitingApproval") return 0;
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

export function headline(waiting: number, working: number): string {
  if (waiting > 0) return `${questions(waiting)} waiting on you`;
  if (working > 0) return `${working} working`;
  return "Nothing running";
}

export function workingCount(agents: AgentCard[], activity: Record<AgentId, Activity>): number {
  return agents.filter((agent) => busy(activity[agent.id])).length;
}

export function said(message: Envelope): string {
  return message.parts
    .flatMap((part) => (part.type === "text" ? [part.text.trim()] : []))
    .filter(Boolean)
    .join("\n\n");
}

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
