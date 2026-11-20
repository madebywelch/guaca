import type { AgentCard, AgentId, Approval, GroupId } from "./types";

export function writable(agents: AgentCard[]): AgentCard[] {
  return agents.filter((agent) => agent.lifecycle !== "terminated");
}

export function waitingQuestion(pending: Approval[]): Approval | undefined {
  return pending
    .filter((request) => request.state === "pending" && request.request.kind === "question")
    .sort((a, b) => a.createdAt - b.createdAt)[0];
}

export function defaultTarget(
  agents: AgentCard[],
  lastActive: Record<AgentId, number>,
  group: GroupId | null,
): AgentCard | undefined {
  const live = writable(agents);
  const newest = (pool: AgentCard[]) =>
    pool
      .filter((agent) => lastActive[agent.id] !== undefined)
      .sort((a, b) => (lastActive[b.id] ?? 0) - (lastActive[a.id] ?? 0))[0];
  const inGroup = group === null ? [] : live.filter((agent) => agent.groupId === group);
  return newest(inGroup) ?? newest(live) ?? inGroup[0] ?? live[0];
}
