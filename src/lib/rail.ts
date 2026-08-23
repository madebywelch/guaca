/**
 * What order the rail draws agents in.
 *
 * Two orders, not one. The arrangement is what the operator dragged into place
 * and it is durable, held as `railOrder` on the card. Activity is a loan: an
 * agent that is working is lifted to the top of its section for as long as it
 * is working, and the place goes back the moment it stops.
 *
 * The rail used to have only the second half, ordered by who spoke last for as
 * long as that stayed true, which meant the list you reached into was the list
 * that had just moved and no arrangement could survive a conversation. Keeping
 * both halves is what makes a drag worth doing: the shape you arranged is the
 * shape the rail returns to.
 *
 * A pin is a third thing and the shortest: it holds a row at the head of its
 * crew, in every view that draws that crew, and it never lends the place back.
 */

import type { Activity, AgentCard, AgentId, GroupId } from "./types";

/**
 * What a dragged row was let go over.
 *
 * Two targets, because the rail says two things. A row is a place in an
 * arrangement, and it carries whether that place is pinned. A group is a crew,
 * and dropping on one asks for membership without saying where in it.
 */
export type DropTarget = { kind: "row"; id: AgentId } | { kind: "group"; id: GroupId };

/**
 * How loudly a state asks for the top of its section.
 *
 * `awaitingApproval` outranks working because it is the one state the operator
 * is the fix for: the agent is parked until they answer, and the request is in a
 * channel they may not have open. Paused scores nothing on purpose. It is not
 * work in progress, it is a row that will not move until someone moves it, and
 * lifting it would hold a place at the top indefinitely.
 */
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
  /** Newest message per agent. Only ever separates two lifted rows. */
  lastActive: Record<AgentId, number>;
  /**
   * Draw the arrangement itself, with nobody lifted.
   *
   * Used while a drag is in progress. Dragging is arranging, so it has to
   * operate on the arrangement: dropping a row below a peer that is only there
   * because it happens to be mid-turn would file it somewhere the operator did
   * not aim at, and the rail would appear to have ignored the gesture the
   * moment that turn ended.
   */
  frozen?: boolean;
}

/**
 * Which band of its section a row sits in. Lower is nearer the top.
 *
 * A pin is the head of the crew, and it is the same head in both views: the
 * overview draws every group with its pins on top, and going inside one draws
 * that group and nothing else. A pin that only held in the overview was a
 * gesture that did nothing to the list the operator was looking at.
 *
 * A pinned row never floats either. It is at the head so it can be found in one
 * glance, and a row that moves when the agent gets busy is the one thing a pin
 * is for stopping.
 */
function bandOf(agent: AgentCard, lift: number): number {
  if (agent.pinned) return 0;
  if (lift > 0) return 1;
  return 2;
}

/**
 * One section of the rail, in the order it is drawn.
 *
 * Self-sufficient about the arrangement: it sorts by `railOrder` rather than
 * trusting the order it was handed, so a caller that filtered a roster cannot
 * accidentally decide where rows go.
 */
export function railOrder(agents: AgentCard[], options: RailOptions): AgentCard[] {
  const { activity, lastActive, frozen = false } = options;

  return agents
    .map((agent, index) => {
      const lift = frozen ? 0 : liftOf(activity[agent.id]);
      return { agent, index, lift, band: bandOf(agent, lift) };
    })
    .sort((a, b) => {
      if (a.band !== b.band) return a.band - b.band;
      // Recency separates two working rows and nothing else. Outside the lifted
      // band it is exactly the ordering this replaced, and letting it through
      // there would reorder a settled rail every time anybody spoke.
      if (a.band === 1) {
        if (a.lift !== b.lift) return b.lift - a.lift;
        const spoke = (lastActive[b.agent.id] ?? 0) - (lastActive[a.agent.id] ?? 0);
        if (spoke !== 0) return spoke;
      }
      return a.agent.railOrder - b.agent.railOrder || a.index - b.index;
    })
    .map((entry) => entry.agent);
}

/**
 * Which row a dragged agent lands in front of, given the row it was dropped on.
 * `null` is the end of the group.
 *
 * Read off two positions rather than measured against a pointer. A row's
 * midpoint is geometry a test cannot see and a hand cannot aim at; the
 * direction the row traveled says which side of the target it belongs on.
 * Dragging down puts it after what it passed, dragging up puts it in front,
 * which is what both look like while the drag is happening.
 *
 * `order` is the section as drawn, so an agent arriving from another group has
 * no position in it and lands in front of the row it was dropped on.
 */
export function landsBefore(
  order: AgentCard[],
  dragged: AgentId,
  onto: AgentId,
): AgentId | null | undefined {
  if (dragged === onto) return undefined;
  const to = order.findIndex((a) => a.id === onto);
  if (to < 0) return undefined;

  const from = order.findIndex((a) => a.id === dragged);
  if (from >= 0 && from < to) return order[to + 1]?.id ?? null;
  return onto;
}

/**
 * Where a row goes when the operator asks for it one place up or down, which is
 * the same move without a mouse. `undefined` means there is nowhere to go.
 */
export function nudgeTarget(
  order: AgentCard[],
  id: AgentId,
  delta: -1 | 1,
): AgentId | null | undefined {
  const at = order.findIndex((a) => a.id === id);
  if (at < 0) return undefined;
  const onto = order[at + delta];
  if (!onto) return undefined;
  return landsBefore(order, id, onto.id);
}
