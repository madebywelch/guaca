import type { AgentCard } from "./types";

export const COMPOST_DAYS = 30;

const DAY = 24 * 60 * 60 * 1000;

export function composted(agents: AgentCard[]): AgentCard[] {
  return agents
    .filter((agent) => agent.discardedAt !== null && agent.discardedAt !== undefined)
    .sort((a, b) => (b.discardedAt ?? 0) - (a.discardedAt ?? 0));
}

export function timeLeft(agent: AgentCard, now: number): string {
  const at = agent.discardedAt;
  if (at === null || at === undefined) return "";

  const left = at + COMPOST_DAYS * DAY - now;
  if (left <= 0) return "going now";

  const days = Math.floor(left / DAY);
  if (days >= 1) return `${days} ${days === 1 ? "day" : "days"} left`;

  const hours = Math.floor(left / (60 * 60 * 1000));
  if (hours >= 1) return `${hours} ${hours === 1 ? "hour" : "hours"} left`;
  return "less than an hour left";
}

export function goingSoon(agent: AgentCard, now: number): boolean {
  const at = agent.discardedAt;
  if (at === null || at === undefined) return false;
  return at + COMPOST_DAYS * DAY - now <= 7 * DAY;
}
