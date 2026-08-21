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
 */

import type { Activity, AgentCard, AgentId, GroupId } from "./types";

/**
 * What a dragged row was let go over.
 *
 * Three targets rather than one, because the rail says three different things.
 * A row is a place in an arrangement. A group is a crew, and dropping on one
 * asks for membership without saying where. Pinned is neither: it is a flag on
 * an agent, drawn as a section so it can be aimed at.
 */
export type DropTarget =
  | { kind: "row"; id: AgentId }
  | { kind: "group"; id: GroupId }
  | { kind: "pinned" };

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
  /**
   * Keep pinned members at the head of this section.
   *
   * For a section that is a whole group, which is what focusing on one draws. In
   * the rail's overview the pinned rows are lifted out into their own section
   * above the groups, so there is nothing to keep at the head of anything.
   */
  pinnedFirst?: boolean;
}

/** Which band of its section a row sits in. Lower is nearer the top. */
function bandOf(agent: AgentCard, lift: number, pinnedFirst: boolean): number {
  if (pinnedFirst && agent.pinned) return 0;
  // A pinned row never floats, wherever it is drawn. It is where it is so it can
  // be found in one glance, and a row that moves when the agent gets busy is the
  // one thing a pin is for stopping.
  if (lift > 0 && !agent.pinned) return 1;
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
  const { activity, lastActive, frozen = false, pinnedFirst = false } = options;

  return agents
    .map((agent, index) => {
      const lift = frozen ? 0 : liftOf(activity[agent.id]);
      return { agent, index, lift, band: bandOf(agent, lift, pinnedFirst) };
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
 * direction the row travelled says which side of the target it belongs on.
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
