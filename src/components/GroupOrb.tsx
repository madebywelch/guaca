import type { CSSProperties, PointerEvent } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { cluster } from "../lib/orb";
import { presenceLabel, presenceNote, presenceOf } from "../lib/presence";
import type { Activity, AgentCard, AgentId, Escalation, Group } from "../lib/types";
import { OrbTag, useOrbTag } from "./OrbTag";

interface Props {
  group: Group;
  members: AgentCard[];
  activity: Record<AgentId, Activity>;
  stuck: readonly Escalation[];
  current: boolean;
  over: boolean;
  held: boolean;
  lands: "before" | "after" | undefined;
  onPress: (event: PointerEvent) => void;
  onOpen: () => void;
  onDragOver: () => void;
  onDragOut: () => void;
}

function percent(fraction: number): string {
  return `${(fraction * 100).toFixed(2)}%`;
}

export function GroupOrb({
  group,
  members,
  activity,
  stuck,
  current,
  over,
  held,
  lands,
  onPress,
  onOpen,
  onDragOver,
  onDragOut,
}: Props) {
  const { seats, rest } = cluster(members);
  const tag = useOrbTag();

  const presence = presenceOf(members, activity, stuck);

  return (
    <button
      type="button"
      className="orb"
      aria-current={current}
      aria-label={presenceLabel(group.name, members.length, presence)}
      title={group.name}
      data-state={presence.working ? "working" : undefined}
      data-over={over ? "true" : undefined}
      data-held={held ? "true" : undefined}
      data-lands={lands}
      onPointerDown={onPress}
      onClick={onOpen}
      onFocus={tag.open}
      onBlur={tag.close}
      onPointerEnter={(event) => {
        tag.open(event);
        onDragOver();
      }}
      onPointerLeave={() => {
        tag.close();
        onDragOut();
      }}
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
      {presence.blocked > 0 && (
        <span className="orb__waiting" aria-hidden="true">
          {presence.blocked}
        </span>
      )}
      {tag.at !== null && (
        <OrbTag name={group.name} note={presenceNote(members.length, presence)} at={tag.at} />
      )}
    </button>
  );
}
