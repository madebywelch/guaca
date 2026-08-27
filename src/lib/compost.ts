import type { AgentCard } from "./types";

/**
 * The compost: agents that were deleted and have not gone yet.
 *
 * Deleting an agent used to be one act, and it took the machines, the memory,
 * the schedule, the sign-ins and every standing permission with it. All of that
 * behind a menu item, on an agent that may have had six months of memory in it.
 * So the destructive half waits: for thirty days the agent is out of the rail
 * and unreachable, exactly as a deleted one has always been, and everything it
 * held privately sits where it was.
 *
 * Nothing here decides anything. Which agents are in the compost, and for how
 * long, is what the runtime wrote on the card; this is the two readings of it
 * the panel and the rail need.
 */

/**
 * How long an agent waits before it is gone for good.
 *
 * Pinned to the Rust constant by `Compost.test.tsx`, which reads both sources.
 * The runtime is what actually deletes, and this number is drawn to the
 * operator as a fact about when that happens: a mirror that has drifted is a
 * promise the app does not keep. Same rule the memory cap follows.
 */
export const COMPOST_DAYS = 30;

const DAY = 24 * 60 * 60 * 1000;

/**
 * Whoever is in the compost, newest first.
 *
 * The order matters more than it looks: what somebody deleted a minute ago by
 * accident is what they came here for, and what has been in there for four
 * weeks is not. Terminated agents with no stamp are not in this list and never
 * come back into it — those are the ones whose wait is already over, and every
 * transcript in the app still draws their names.
 */
export function composted(agents: AgentCard[]): AgentCard[] {
  return agents
    .filter((agent) => agent.discardedAt !== null && agent.discardedAt !== undefined)
    .sort((a, b) => (b.discardedAt ?? 0) - (a.discardedAt ?? 0));
}

/**
 * What is left of an agent's thirty days, in the coarsest unit still true.
 *
 * Days until the last one, then hours, because a deadline four weeks out and a
 * deadline this afternoon are read differently and "1d" says both. Rounded
 * down: an agent with a few hours left is on its last day, and telling somebody
 * they have a day when they have three hours is the error that costs them the
 * agent.
 *
 * Past the deadline it says the wait is over rather than counting negatives.
 * The sweep runs hourly, so a row can outlive its deadline by up to an hour,
 * and a panel drawing "-0d" for that hour reads as something broken.
 */
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

/**
 * Whether an agent is close enough to going that the row should say so
 * loudly.
 *
 * A week, because that is the horizon somebody who opens this panel once a
 * fortnight can still act inside. Below it the row is marked; above it the
 * count of days is all the warning that is warranted, and marking everything
 * marks nothing.
 */
export function goingSoon(agent: AgentCard, now: number): boolean {
  const at = agent.discardedAt;
  if (at === null || at === undefined) return false;
  return at + COMPOST_DAYS * DAY - now <= 7 * DAY;
}
