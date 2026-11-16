import type { AgentCard, AgentId, Approval, GroupId } from "./types";

/**
 * What the field at the top of the window does with what was typed, decided
 * before anything is sent. Pure, so the three decisions it makes (who it goes
 * to, whether it answers the question that is waiting, and whether it names
 * somebody else with an `@`) are tested here rather than through a window.
 */

/** The live agents the field can write to: not deleted, in roster order. */
export function writable(agents: AgentCard[]): AgentCard[] {
  return agents.filter((agent) => agent.lifecycle !== "terminated");
}

/**
 * The question Tab answers: the oldest pending one. A permission is not one,
 * because it is answered with a verdict rather than with words, and a field
 * that turned a typed sentence into Allow would be granting by accident.
 */
export function waitingQuestion(pending: Approval[]): Approval | undefined {
  return pending
    .filter((request) => request.state === "pending" && request.request.kind === "question")
    .sort((a, b) => a.createdAt - b.createdAt)[0];
}

/**
 * Who the field writes to before the operator picks: whoever was talked to
 * last in the group the operator is in, else whoever was talked to last at all,
 * else the first agent. Never a deleted one.
 */
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

/**
 * An `@Name` at the very start addresses that agent instead of the picked one.
 *
 * Only at the start, and only a whole name followed by a space: an `@` in the
 * middle of a sentence is a mention the message carries, which is the
 * composer's business and the runtime's, not a change of recipient. The
 * longest name wins, so "@Ines Marie" is not read as Ines when both exist.
 */
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

/**
 * The agent an `@Name` names when it is the whole of what was typed: a
 * recipient with no message, which the field takes as a pick rather than
 * sending the name on its own as the message.
 */
export function namedAlone(text: string, agents: AgentCard[]): AgentCard | undefined {
  const typed = text.trim();
  if (!typed.startsWith("@")) return undefined;
  const name = typed.slice(1).toLowerCase();
  return writable(agents).find((agent) => agent.name.toLowerCase() === name);
}
