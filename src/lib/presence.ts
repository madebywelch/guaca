/**
 * What a crew is, in the two marks a circle the size of a thumbnail can carry.
 *
 * The group column is on screen wherever the operator is, and it is the only
 * thing about the crews they are not looking at that ever will be. So it has to
 * answer one question without being clicked: is anything over there my problem.
 *
 * Two marks, and they are deliberately not one:
 *
 *   a count    things in this crew that a person is the fix for: turns parked
 *              on the operator, and agents that have stopped and said so. A
 *              number, because "three" and "one" are different amounts of work
 *              and a dot says neither.
 *   a ring     somebody here is working. No number: how many agents are
 *              mid-turn is not a thing anybody acts on, and a second number
 *              beside the first would make the first harder to read.
 *
 * The same two states the menu bar's glyph already distinguishes, and for the
 * same reason. `menubar.rs` reports them for the workspace while the window is
 * shut; this reports them per crew while it is open. One rule, two granularities:
 * see *The menu bar is Guaca with the window shut* in `docs/WORKSPACE.md`.
 *
 * What is deliberately absent is unread. Discord's rail carries a dot for
 * messages you have not seen, and Guaca has no idea which messages those are:
 * nothing records what the operator has read. A dot derived from "this channel
 * is not the open one" would be lit on every crew but one, forever, which is a
 * mark that says nothing. Unread is a persisted per-channel marker and a
 * migration, not a badge, and it is not in this change.
 */

import type { Activity, AgentCard, AgentId, Escalation } from "./types";

export interface Presence {
  /**
   * What in this crew is waiting on the operator: turns parked on a request,
   * plus escalations nobody has cleared.
   *
   * One number over two sources, because the circle is answering one question
   * and the question is "is anything over there mine". They are worth
   * distinguishing on the desk, which can afford two cards, and not here, where
   * the whole statement is a digit on a circle the size of a thumbnail.
   */
  blocked: number;
  /** Anyone here is mid-turn or holding queued work. */
  working: boolean;
}

export const QUIET: Presence = { blocked: 0, working: false };

/**
 * What one crew amounts to right now.
 *
 * Parked turns are counted off `activity` rather than off the pending requests,
 * because that half is a question about agents. The two are one to one by
 * construction: an agent runs one turn at a time and a turn parks on one
 * request, so a crew with two parked agents has two rows in `approvals`.
 *
 * Escalations are not, and cannot be. Nothing parks on one -- the turn that
 * raised it ended -- so the agent holding one is idle, or working on something
 * else, and its activity says so correctly. The list is the only place it
 * exists, which is why it is an argument rather than something read off the
 * roster. An agent parked on a request while an older escalation of its own is
 * still open counts twice, and that is right: they are two things to deal with,
 * and the desk has two cards for them.
 *
 * Every state is named rather than defaulted, and that is what makes this safe
 * to leave alone. A variant added to the runtime and not weighed here would
 * fall to the default and count as nothing, so a crew in the new state would
 * draw as idle: the one reading that makes the column not worth a glance. Named
 * exhaustively, the same addition fails the build instead.
 */
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
      // Neither is work in progress. Idle is nothing happening, and paused is a
      // row that will not move until somebody moves it: a ring lit for one
      // would never go out. `liftOf` gives both the same nothing, for the same
      // reason, which is why a paused agent is not lifted up its section.
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

/**
 * The crew in words, without its name: how big it is and what it is doing.
 *
 * The one place the two marks are ranked rather than both drawn. The circle has
 * a corner and a ring and can carry each on its own channel; a sentence is one
 * channel, and "two turns waiting on you, and working" buries the half that
 * needs a person under the half that does not.
 *
 * Split out because the sentence is now said twice, to two audiences that must
 * not be given different sentences: the label below, and the line the column
 * draws under a crew's name when it is pointed at. A tag that said "3 agents"
 * where the label said "working" would be the same circle described two ways,
 * and only one of them could be right.
 */
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

/**
 * What the circle says out loud, for anything that cannot see it.
 *
 * A number in a corner is invisible to a screen reader unless the control says
 * it, and "Sales" alone is a crew a blind operator has no reason to open.
 */
export function presenceLabel(name: string, count: number, presence: Presence): string {
  return `${name}, ${presenceNote(count, presence)}`;
}
