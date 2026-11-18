import type { Activity, AgentCard, AgentId, GroupId } from "./types";

export type DropTarget = { kind: "row"; id: AgentId } | { kind: "group"; id: GroupId };

export function liftOf(activity: Activity | undefined): number {
  switch (activity?.state) {
    case "awaitingApproval":
      return 3;
    case "thinking":
      return 2;
    case "queued":
      return 1;
    default:
      return 0;
  }
}

export interface RailOptions {
  activity: Record<AgentId, Activity>;
  lastActive: Record<AgentId, number>;
  frozen?: boolean;
}

function bandOf(agent: AgentCard, lift: number): number {
  if (agent.pinned) return 0;
  if (lift > 0) return 1;
  return 2;
}

export function railOrder(agents: AgentCard[], options: RailOptions): AgentCard[] {
  const { activity, lastActive, frozen = false } = options;

  return agents
    .map((agent, index) => {
      const lift = frozen ? 0 : liftOf(activity[agent.id]);
      return { agent, index, lift, band: bandOf(agent, lift) };
    })
    .sort((a, b) => {
      if (a.band !== b.band) return a.band - b.band;
      if (a.band === 1) {
        if (a.lift !== b.lift) return b.lift - a.lift;
        const spoke = (lastActive[b.agent.id] ?? 0) - (lastActive[a.agent.id] ?? 0);
        if (spoke !== 0) return spoke;
      }
      return a.agent.railOrder - b.agent.railOrder || a.index - b.index;
    })
    .map((entry) => entry.agent);
}

export function numbered(agents: AgentCard[], groups: readonly { id: GroupId }[]): AgentCard[] {
  const live = agents.filter((agent) => agent.lifecycle !== "terminated");
  const still = { activity: {}, lastActive: {}, frozen: true };
  const placed = groups.flatMap((group) =>
    railOrder(
      live.filter((agent) => agent.groupId === group.id),
      still,
    ),
  );
  return [...placed, ...live.filter((agent) => !placed.includes(agent))];
}

export function landsBefore(
  order: readonly { id: string }[],
  dragged: string,
  onto: string,
): string | null | undefined {
  if (dragged === onto) return undefined;
  const to = order.findIndex((a) => a.id === onto);
  if (to < 0) return undefined;

  const from = order.findIndex((a) => a.id === dragged);
  if (from >= 0 && from < to) return order[to + 1]?.id ?? null;
  return onto;
}
