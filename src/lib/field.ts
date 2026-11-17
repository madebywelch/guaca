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

export function addressed(
  text: string,
  agents: AgentCard[],
): { agent: AgentCard; text: string } | undefined {
  if (!text.startsWith("@")) return undefined;
  const rest = text.slice(1);
  const match = writable(agents)
    .filter((agent) => {
      const name = agent.name.toLowerCase();
      const head = rest.slice(0, name.length).toLowerCase();
      return head === name && /^\s/.test(rest.slice(name.length));
    })
    .sort((a, b) => b.name.length - a.name.length)[0];
  if (!match) return undefined;
  const body = rest.slice(match.name.length).trim();
  return body ? { agent: match, text: body } : undefined;
}

export function namedAlone(text: string, agents: AgentCard[]): AgentCard | undefined {
  const typed = text.trim();
  if (!typed.startsWith("@")) return undefined;
  const name = typed.slice(1).toLowerCase();
  return writable(agents).find((agent) => agent.name.toLowerCase() === name);
}
