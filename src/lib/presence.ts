import type { Activity, AgentCard, AgentId, Escalation } from "./types";

export interface Presence {
  blocked: number;
  working: boolean;
}

export const QUIET: Presence = { blocked: 0, working: false };

export function presenceOf(
  members: AgentCard[],
  activity: Record<AgentId, Activity>,
  stuck: readonly Escalation[],
): Presence {
  let blocked = 0;
  let working = false;

  const here = new Set(members.map((member) => member.id));
  for (const one of stuck) {
    if (here.has(one.agentId)) blocked += 1;
  }

  for (const member of members) {
    const state = activity[member.id];
    if (state === undefined) continue;

    switch (state.state) {
      case "awaitingApproval":
        blocked += 1;
        break;
      case "thinking":
      case "queued":
        working = true;
        break;
      case "idle":
      case "paused":
        break;
      default: {
        const unweighed: never = state;
        void unweighed;
      }
    }
  }

  return { blocked, working };
}

export function presenceNote(count: number, presence: Presence): string {
  const size = count === 1 ? "1 agent" : `${count} agents`;
  if (presence.blocked > 0) {
    const waiting =
      presence.blocked === 1 ? "1 turn waiting on you" : `${presence.blocked} turns waiting on you`;
    return `${size}, ${waiting}`;
  }
  if (presence.working) return `${size}, working`;
  return size;
}

export function presenceLabel(name: string, count: number, presence: Presence): string {
  return `${name}, ${presenceNote(count, presence)}`;
}
