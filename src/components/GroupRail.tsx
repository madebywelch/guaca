import type { DropTarget } from "../lib/rail";
import type { Activity, AgentCard, AgentId, Group, GroupId } from "../lib/types";
import { GroupOrb } from "./GroupOrb";

interface Props {
  groups: Group[];
  /** Every live agent, across every crew. */
  agents: AgentCard[];
  activity: Record<AgentId, Activity>;
  /** The crew the rail is inside, or `null` for all of them. */
  focused: GroupId | null;
  onFocus: (id: GroupId | null) => void;
  /** Whether a dragged row would land on this target if it were dropped now. */
  isOver: (target: DropTarget) => boolean;
  onDragOver: (target: DropTarget) => void;
  onDragOut: () => void;
}

/**
 * The crews, as a column of their own on the far left.
 *
 * This was a strip of circles inside the rail, wrapping onto a second line at
 * five crews and a third at nine. It did not overflow or scroll: it ate the
 * rail's own height, so a workspace with a dozen crews spent the top third of
 * its agent list on the navigation for it. Four was a hard wall nobody chose.
 *
 * A column has the axis to spare, and it buys two things the strip could not.
 * The crews are on screen no matter where the operator is, including while the
 * rail is inside one of them, so the badge on a circle is the only permanent
 * statement in the app about the crews you are not looking at: see
 * `lib/presence.ts` for what it may say. And "which crew am I in" stops being
 * something the rail has to give a heading to, because the answer is a lit
 * circle in a fixed place.
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
  focused,
  onFocus,
  isOver,
  onDragOver,
  onDragOut,
}: Props) {
  if (groups.length < 2) return null;

  return (
    <nav className="grail" aria-label="Groups">
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
      >
        <span className="orb__ring">
          <span className="orb__all-count">{agents.length}</span>
        </span>
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
            current={focused === group.id}
            over={isOver({ kind: "group", id: group.id })}
            onOpen={() => onFocus(focused === group.id ? null : group.id)}
            onDragOver={() => onDragOver({ kind: "group", id: group.id })}
            onDragOut={onDragOut}
          />
        ))}
      </div>
    </nav>
  );
}
