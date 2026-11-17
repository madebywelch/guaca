import {
  type CSSProperties,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
  useEffect,
  useRef,
  useState,
} from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { type DropTarget, landsBefore } from "../lib/rail";
import { reaches } from "../lib/reach";
import type { Activity, AgentCard, AgentId, Escalation, Group, GroupId } from "../lib/types";
import { GroupOrb } from "./GroupOrb";
import { OrbTag, useOrbTag } from "./OrbTag";

interface Props {
  groups: Group[];
  agents: AgentCard[];
  activity: Record<AgentId, Activity>;
  stuck: readonly Escalation[];
  focused: GroupId | null;
  onFocus: (id: GroupId | null) => void;
  isOver: (target: DropTarget) => boolean;
  onDragOver: (target: DropTarget) => void;
  onDragOut: () => void;
  dragging: boolean;
  held: GroupId | null;
  onPress: (id: GroupId, event: ReactPointerEvent) => void;
  listRef: RefObject<HTMLDivElement | null>;
}

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
  held,
  onPress,
  listRef,
}: Props) {
  const all = useOrbTag();
  const zone = useRef<HTMLSpanElement>(null);
  const slab = useRef<HTMLDivElement>(null);
  const [near, setNear] = useState(false);

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
  }, [dragging]);

  if (groups.length < 2) return null;

  const current = groups.find((group) => group.id === focused);
  const face = current ? agents.find((agent) => agent.groupId === current.id) : undefined;

  const lands = (id: GroupId): "before" | "after" | undefined => {
    if (held === null || !isOver({ kind: "group", id })) return undefined;
    const before = landsBefore(groups, held, id);
    if (before === undefined) return undefined;
    return before === id ? "before" : "after";
  };

  return (
    <nav
      className="grail"
      aria-label="Groups"
      data-out={near ? "true" : undefined}
      data-dragging={dragging ? "true" : undefined}
    >
      <span className="grail__reach" aria-hidden="true" ref={zone} />

      <span className="grail__mark" aria-hidden="true">
        {face && (
          <span
            className="orb__face"
            style={{ "--seat-x": "50%", "--seat-y": "50%", "--seat-size": "68%" } as CSSProperties}
          >
            <AgentAvatar
              avatar={face.avatar}
              color={face.color}
              size="xs"
              seed={face.id}
              lifecycle={face.lifecycle}
            />
          </span>
        )}
      </span>

      <div className="grail__slab" ref={slab}>
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

        <div className="grail__list" ref={listRef}>
          {groups.map((group) => (
            <GroupOrb
              key={group.id}
              group={group}
              members={agents.filter((a) => a.groupId === group.id)}
              activity={activity}
              stuck={stuck}
              current={focused === group.id}
              over={held === null && isOver({ kind: "group", id: group.id })}
              held={held === group.id}
              lands={lands(group.id)}
              onPress={(event) => onPress(group.id, event)}
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
