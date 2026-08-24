import type { CSSProperties } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { cluster } from "../lib/orb";
import { presenceLabel, presenceOf } from "../lib/presence";
import type { Activity, AgentCard, AgentId, Group } from "../lib/types";

interface Props {
  group: Group;
  /** Live members, in the order the rail arranged them. */
  members: AgentCard[];
  activity: Record<AgentId, Activity>;
  /** Whether the rail is currently looking inside this group. */
  current: boolean;
  /** Whether a dragged row would land here if it were dropped now. */
  over: boolean;
  onOpen: () => void;
  onDragOver: () => void;
  onDragOut: () => void;
}

/** A seat's fraction of the ring, as a length the stylesheet can use. */
function percent(fraction: number): string {
  return `${(fraction * 100).toFixed(2)}%`;
}

/**
 * A group, small enough to sit in a column and be aimed at.
 *
 * Faces rather than a name, because a crew is recognized by who is in it long
 * before its name is read. How they stand is the crew's size: `lib/orb.ts` owns
 * the seating and says why. The name is not drawn at all here, because the
 * column is four rem wide and a name cut to fit it is a name nobody can read;
 * it is on the tooltip, on the label, and heading the crew column the moment
 * this is clicked.
 *
 * Two jobs in one control, which is why it is a circle and not a row in a menu.
 * Clicking opens the group. Dropping an agent on it puts the agent in the group,
 * so the shortest gesture for moving somebody between crews is the one that also
 * says which crew, and it is on screen the whole time you are dragging.
 */
export function GroupOrb({
  group,
  members,
  activity,
  current,
  over,
  onOpen,
  onDragOver,
  onDragOut,
}: Props) {
  const { seats, rest } = cluster(members);

  // The two states worth interrupting a glance for, and why they are not one
  // mark: `lib/presence.ts`. This column is the only thing on screen about the
  // crews the operator is not looking at, so what it can say is all they get.
  const presence = presenceOf(members, activity);

  return (
    <button
      type="button"
      className="orb"
      aria-current={current}
      aria-label={presenceLabel(group.name, members.length, presence)}
      title={group.name}
      data-state={presence.working ? "working" : undefined}
      data-over={over ? "true" : undefined}
      onClick={onOpen}
      onPointerEnter={onDragOver}
      onPointerLeave={onDragOut}
    >
      <span className="orb__ring">
        <span className="orb__faces">
          {seats.map((seat) => {
            const member = seat.of;
            return (
              <span
                key={member.id}
                className="orb__face"
                style={
                  {
                    "--seat-x": percent(seat.x),
                    "--seat-y": percent(seat.y),
                    "--seat-size": percent(seat.size),
                    "--seat-tilt": `${seat.tilt}deg`,
                  } as CSSProperties
                }
              >
                <AgentAvatar
                  avatar={member.avatar}
                  color={member.color}
                  size="xs"
                  seed={member.id}
                  lifecycle={member.lifecycle}
                  title={member.name}
                />
              </span>
            );
          })}
          {seats.length === 0 && <span className="orb__none">·</span>}
        </span>
        {rest > 0 && <span className="orb__more">+{rest}</span>}
      </span>
      {/* A number rather than a dot, because three parked turns and one are
          different amounts of work. Outside the ring for the same reason the
          overflow count is: a crew with a seat in every quarter has nowhere
          inside it for an opaque chip that is not somebody's face. */}
      {presence.blocked > 0 && (
        <span className="orb__waiting" aria-hidden="true">
          {presence.blocked}
        </span>
      )}
    </button>
  );
}
