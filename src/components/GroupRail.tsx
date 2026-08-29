import { useEffect, useRef, useState } from "react";

import type { DropTarget } from "../lib/rail";
import { reaches } from "../lib/reach";
import type { Activity, AgentCard, AgentId, Escalation, Group, GroupId } from "../lib/types";
import { GroupOrb } from "./GroupOrb";
import { OrbTag, useOrbTag } from "./OrbTag";

interface Props {
  groups: Group[];
  /** Every live agent, across every crew. */
  agents: AgentCard[];
  activity: Record<AgentId, Activity>;
  /** Every open escalation, so a circle can say a crew has stopped. */
  stuck: readonly Escalation[];
  /** The crew the rail is inside, or `null` for all of them. */
  focused: GroupId | null;
  onFocus: (id: GroupId | null) => void;
  /** Whether a dragged row would land on this target if it were dropped now. */
  isOver: (target: DropTarget) => boolean;
  onDragOver: (target: DropTarget) => void;
  onDragOut: () => void;
  /**
   * Whether the operator is currently holding an agent.
   *
   * Holds the column where it is rather than holding it out. A drop onto a
   * circle is the one thing this column is load-bearing for, so a column
   * already reached for must not slide away as the hand carries the row back
   * across the app; a column that came out for every drag would instead cover
   * the left edge of every row in the rail during a reorder, which is what
   * most drags are. `lib/reach.ts` has the rest of it.
   */
  dragging: boolean;
}

/**
 * The crews, as a column of their own on the far left, out of the way until
 * it is reached for.
 *
 * This was a strip of circles inside the rail, wrapping onto a second line at
 * five crews and a third at nine. It did not overflow or scroll: it ate the
 * rail's own height, so a workspace with a dozen crews spent the top third of
 * its agent list on the navigation for it. Four was a hard wall nobody chose.
 *
 * A column has the axis to spare, and it buys two things the strip could not.
 * The crews are reachable no matter where the operator is, including while the
 * rail is inside one of them, so the badge on a circle is the only permanent
 * statement in the app about the crews you are not looking at: see
 * `lib/presence.ts` for what it may say. And "which crew am I in" stops being
 * something the rail has to give a heading to, because the answer is a lit
 * circle in a fixed place.
 *
 * Standing open, that column charged every window four rem of width for a
 * choice most operators make a few times a day, in front of the rail that is
 * read constantly. So it is a band at the edge of the window that slides out
 * when the pointer comes at it, over the rail rather than pushing it: nothing
 * reflows, the rail keeps its measure, and the gesture is the one somebody
 * heading for the left edge is already making. `lib/reach.ts` owns when, and
 * the two thresholds it uses and why there are two.
 *
 * Two things bring it out and none of them is a click: proximity, and focus
 * inside it, which is what makes the column reachable from the keyboard, since
 * tabbing into a circle nobody can see is a selection nobody can read. A drag
 * brings it out for neither, and only keeps it wherever it already was.
 *
 * Still absent while there is one group, which is the state most workspaces are
 * in and the same rule the strip had. A column offering a choice of one is a
 * drop target that cannot move anybody anywhere, and with one crew the rail is
 * that crew: every row of it is already on screen, saying the same thing the
 * badge would.
 */
export function GroupRail({
  groups,
  agents,
  activity,
  stuck,
  focused,
  onFocus,
  isOver,
  onDragOver,
  onDragOut,
  dragging,
}: Props) {
  const all = useOrbTag();
  const zone = useRef<HTMLSpanElement>(null);
  const slab = useRef<HTMLDivElement>(null);
  const [near, setNear] = useState(false);

  // On the window rather than on a strip the pointer can enter, because a strip
  // wide enough to be aimed at overlays the left edge of every agent row behind
  // it and swallows the click. Both boxes are measured on each movement rather
  // than cached: the column's own edge is the thing that moves, and the zone's
  // is a length in the stylesheet that changes with the interface scale.
  useEffect(() => {
    const move = (event: PointerEvent) => {
      const reach = zone.current?.getBoundingClientRect();
      const column = slab.current?.getBoundingClientRect();
      if (!reach || !column) return;
      setNear((open) =>
        reaches(open, { x: event.clientX, y: event.clientY }, { reach, column }, dragging),
      );
    };
    window.addEventListener("pointermove", move);
    return () => window.removeEventListener("pointermove", move);
    // Twice a drag, which is what it costs to have the rule read one boolean
    // rather than a ref that says what it was on the last frame.
  }, [dragging]);

  if (groups.length < 2) return null;

  return (
    <nav className="grail" aria-label="Groups" data-out={near ? "true" : undefined}>
      {/* The proximity zone, as a box. It carries no pixels and cannot be
          clicked; what it is for is that its size and its distance from the top
          of the window are lengths, and every length in this app is named in
          one stylesheet rather than written into a component. */}
      <span className="grail__reach" aria-hidden="true" ref={zone} />

      <div className="grail__slab" ref={slab}>
        {/* Everybody, which is the view the rail had before crews were places you
            could be inside. A count rather than the word, because the label
            already says "all" and a group's name can be anything, including
            "everyone". */}
        <button
          type="button"
          className="orb orb--all"
          aria-current={focused === null}
          aria-label={`All groups, ${agents.length} agents`}
          title="All groups"
          onClick={() => onFocus(null)}
          onPointerEnter={all.open}
          onPointerLeave={all.close}
          onFocus={all.open}
          onBlur={all.close}
        >
          <span className="orb__ring">
            <span className="orb__all-count">{agents.length}</span>
          </span>
          {all.at !== null && (
            <OrbTag name="All groups" note={`${agents.length} agents`} at={all.at} />
          )}
        </button>

        <span className="grail__rule" aria-hidden="true" />

        {/* Scrolls rather than wraps, which is the whole point of turning this
            upright: a workspace can hold more crews than fit down the side of a
            window, and the ones that do not fit have to be reachable rather than
            folded into a second column. */}
        <div className="grail__list">
          {groups.map((group) => (
            <GroupOrb
              key={group.id}
              group={group}
              members={agents.filter((a) => a.groupId === group.id)}
              activity={activity}
              stuck={stuck}
              current={focused === group.id}
              over={isOver({ kind: "group", id: group.id })}
              // Clicking the crew the rail is already inside does nothing. It
              // used to take the rail back out to the overview, which made the
              // circle a toggle: a double-click on a crew went in and straight
              // back out again, and a click on the crew you were already in put
              // you somewhere you had not asked to go. The way out is the
              // circle at the top of this column, above the rule, which is on
              // screen for the whole of that gesture. A control with its own
              // way out beside it does not need to be one.
              onOpen={() => onFocus(group.id)}
              onDragOver={() => onDragOver({ kind: "group", id: group.id })}
              onDragOut={onDragOut}
            />
          ))}
        </div>
      </div>
    </nav>
  );
}
