import type { CSSProperties } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { cluster } from "../lib/orb";
import { liftOf } from "../lib/rail";
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
 * A group, small enough to sit in a strip and be aimed at.
 *
 * Faces rather than a name, because a crew is recognised by who is in it long
 * before its name is read. How they stand is the crew's size: `lib/orb.ts` owns
 * the seating and says why. The name is still there for anything that reads
 * rather than looks: the label, the tooltip, and the heading you get after
 * clicking.
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

  // The two states worth interrupting a glance for. Asking is louder because
  // the agent is parked until the operator answers, and after focusing on one
  // group the strip is the only place the other crews are still visible at all.
  const asking = members.some((m) => activity[m.id]?.state === "awaitingApproval");
  const working = members.some((m) => liftOf(activity[m.id]) > 0);

  const state = asking ? "asking" : working ? "working" : undefined;
  const busy = asking ? "someone needs you" : working ? "working" : null;

  return (
    <button
      type="button"
      className="orb"
      aria-current={current}
      aria-label={`${group.name}, ${members.length} ${members.length === 1 ? "agent" : "agents"}${
        busy ? `, ${busy}` : ""
      }`}
      title={group.name}
      data-state={state}
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
      <span className="orb__name">{group.name}</span>
    </button>
  );
}
