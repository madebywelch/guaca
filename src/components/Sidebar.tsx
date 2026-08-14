import { useLayoutEffect, useRef, useState } from "react";

import { AgentAvatar, type Look } from "../avatars/AgentAvatar";
import { FLIGHT_MS, roleOf, usePulseChoreography } from "../lib/choreography";
import { ACTIVITY_CHANNEL, useLiveAgents, useStore } from "../lib/store";
import { relativeTime, useNow } from "../lib/time";
import type { Activity, AgentCard, AgentId } from "../lib/types";

interface Props {
  onNewAgent: () => void;
  onEditAgent: (agent: AgentCard) => void;
  onOpenSettings: () => void;
}

function prefersReducedMotion(): boolean {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

export function Sidebar({ onNewAgent, onEditAgent, onOpenSettings }: Props) {
  const agents = useLiveAgents();
  const activity = useStore((s) => s.activity);
  const lastActive = useStore((s) => s.lastActive);
  const selected = useStore((s) => s.selected);
  const select = useStore((s) => s.select);
  const pulses = useStore((s) => s.pulses);
  const dismissPulse = useStore((s) => s.dismissPulse);
  const now = useNow();

  const listRef = useRef<HTMLDivElement>(null);
  const rowRefs = useRef(new Map<AgentId, HTMLButtonElement>());
  const [rowCenters, setRowCenters] = useState<Map<AgentId, number>>(new Map());
  const previousTops = useRef(new Map<AgentId, number>());

  // Messages arrive in bursts; this spaces them out so each throw is watchable.
  const { staged, inFlight } = usePulseChoreography(pulses, dismissPulse);

  // One layout pass does two jobs: slide rows that moved, and record where
  // every row ended up so the travelling message knows where to fly.
  useLayoutEffect(() => {
    const measure = () => {
      if (!listRef.current) return;
      const centers = new Map<AgentId, number>();
      for (const [id, node] of rowRefs.current) {
        centers.set(id, node.offsetTop + node.offsetHeight / 2);
      }
      // Bail out when nothing moved. The observer fires on any layout change,
      // and a fresh Map every time would re-render the rail continuously.
      setRowCenters((current) => {
        if (
          current.size === centers.size &&
          [...centers].every(([id, y]) => current.get(id) === y)
        ) {
          return current;
        }
        return centers;
      });
    };

    // FLIP: the rows are already in their new places, so put each one back
    // where it was and let CSS carry it forward. Reordering instantly is
    // disorienting when several agents move at once.
    const tops = new Map<AgentId, number>();
    for (const [id, node] of rowRefs.current) tops.set(id, node.offsetTop);

    if (!prefersReducedMotion()) {
      for (const [id, node] of rowRefs.current) {
        const before = previousTops.current.get(id);
        const after = tops.get(id);
        if (before === undefined || after === undefined || before === after) continue;

        node.dataset.sliding = "false";
        node.style.transition = "none";
        node.style.transform = `translateY(${before - after}px)`;
        requestAnimationFrame(() => {
          node.dataset.sliding = "true";
          node.style.transition = "";
          node.style.transform = "";
        });
      }
    }
    previousTops.current = tops;

    measure();
    const observer = new ResizeObserver(measure);
    if (listRef.current) observer.observe(listRef.current);
    return () => observer.disconnect();
  }, [agents]);

  /** Which way an agent should turn to face a peer. */
  const facing = (self: AgentId, other: AgentId | undefined): Look => {
    if (!other) return null;
    const mine = rowCenters.get(self);
    const theirs = rowCenters.get(other);
    if (mine === undefined || theirs === undefined) return null;
    return theirs > mine ? "down" : "up";
  };

  /** What the rail shows beside a name. */
  const meta = (
    id: AgentId,
    state: Activity | undefined,
  ): { text: string; kind: string | undefined } => {
    switch (state?.state) {
      case "thinking":
        return { text: "typing", kind: "thinking" };
      case "queued":
        return { text: `${state.depth} queued`, kind: "queued" };
      case "paused":
        return { text: "paused", kind: "paused" };
      default: {
        // Falling back to a last-active time keeps the column occupied. An
        // empty slot the instant an agent stops typing reads as the row
        // breaking rather than the agent finishing.
        const at = lastActive[id];
        return { text: at ? relativeTime(at, now) : "", kind: undefined };
      }
    }
  };

  return (
    <nav className="rail" aria-label="Agents">
      <div className="rail__brand" data-tauri-drag-region>
        <span className="rail__wordmark">Guac</span>
      </div>

      <button
        type="button"
        className="btn btn--rail"
        data-current={selected === ACTIVITY_CHANNEL}
        onClick={() => void select(ACTIVITY_CHANNEL)}
      >
        <span aria-hidden="true" className="rail__hash">
          #
        </span>
        activity
      </button>

      <div className="rail__section">
        <span>Agents</span>
        <span>{agents.length}</span>
      </div>

      {/* The wire lives on this wrapper rather than on the scroll container, so
          it runs the full height of the rail down to the footer instead of
          stopping wherever the list happens to end. */}
      <div className="rail__body">
        <div className="rail__list" ref={listRef}>
          {inFlight.map((pulse) => {
            const from = rowCenters.get(pulse.from);
            const to = rowCenters.get(pulse.to);
            if (from === undefined || to === undefined) return null;
            return (
              <span
                key={pulse.id}
                className="pulse"
                style={
                  {
                    "--pulse-from": `${from}px`,
                    "--pulse-to": `${to}px`,
                    "--pulse-color": pulse.color,
                    "--pulse-duration": `${FLIGHT_MS}ms`,
                  } as React.CSSProperties
                }
              />
            );
          })}

          {agents.map((agent) => {
            const role = roleOf(staged, agent.id);
            const label = meta(agent.id, activity[agent.id]);

            return (
              <button
                key={agent.id}
                type="button"
                ref={(node) => {
                  if (node) rowRefs.current.set(agent.id, node);
                  else rowRefs.current.delete(agent.id);
                }}
                className="agent-row"
                aria-current={selected === agent.id}
                data-lifecycle={agent.lifecycle}
                style={{ "--accent": agent.color } as React.CSSProperties}
                onClick={() => void select(agent.id)}
                onDoubleClick={() => onEditAgent(agent)}
              >
                <AgentAvatar
                  avatar={agent.avatar}
                  color={agent.color}
                  activity={activity[agent.id]}
                  lifecycle={agent.lifecycle}
                  seed={agent.id}
                  gesture={role.gesture}
                  look={facing(agent.id, role.facing ?? undefined)}
                  says={role.says}
                />
                <span className="agent-row__name">{agent.name}</span>
                <span className="agent-row__meta" data-state={label.kind}>
                  {label.text}
                </span>
              </button>
            );
          })}

          {agents.length === 0 && <p className="rail__empty">No agents yet.</p>}
        </div>
      </div>

      <div className="rail__foot">
        <button type="button" className="btn btn--rail" onClick={onNewAgent}>
          <span aria-hidden="true" className="rail__hash">
            +
          </span>
          New agent
        </button>
        <button type="button" className="btn btn--rail" onClick={onOpenSettings}>
          <span aria-hidden="true" className="rail__hash">
            ⚙
          </span>
          Settings
        </button>
      </div>
    </nav>
  );
}
