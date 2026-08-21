import { AgentAvatar } from "../avatars/AgentAvatar";
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

/** How many faces fit in the circle before it starts counting instead. */
const FACES = 4;

/**
 * A group, small enough to sit in a strip and be aimed at.
 *
 * Faces rather than a name, because a crew is recognised by who is in it long
 * before its name is read, and four of these have to fit across a rail that is
 * 15.5rem wide. The name is still there for anything that reads rather than
 * looks: the label, the tooltip, and the heading you get after clicking.
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
  const faces = members.slice(0, FACES);
  const rest = members.length - faces.length;

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
        <span className="orb__faces" data-count={faces.length}>
          {faces.map((member) => (
            <AgentAvatar
              key={member.id}
              avatar={member.avatar}
              color={member.color}
              size="xs"
              seed={member.id}
              lifecycle={member.lifecycle}
              title={member.name}
            />
          ))}
          {faces.length === 0 && <span className="orb__none">·</span>}
        </span>
        {rest > 0 && <span className="orb__more">+{rest}</span>}
      </span>
      <span className="orb__name">{group.name}</span>
    </button>
  );
}
