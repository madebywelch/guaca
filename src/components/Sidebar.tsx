import { useLayoutEffect, useRef, useState } from "react";

import { AgentAvatar, type Look } from "../avatars/AgentAvatar";
import { FLIGHT_MS, roleOf, usePulseChoreography } from "../lib/choreography";
import { ACTIVITY_CHANNEL, useLiveAgents, useStore } from "../lib/store";
import { relativeTime, useNow } from "../lib/time";
import type { Activity, AgentCard, AgentId, Group } from "../lib/types";
import { TokenMeter } from "./TokenMeter";

interface Props {
  onNewAgent: () => void;
  onEditAgent: (agent: AgentCard) => void;
  onNewGroup: () => void;
  onEditGroup: (group: Group) => void;
  onOpenSettings: () => void;
  onOpenSearch: () => void;
  /** Where the operator right-clicked, and on whom. */
  onOpenMenu: (agent: AgentCard, at: { x: number; y: number }) => void;
}

function prefersReducedMotion(): boolean {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

/**
 * How this machine writes the find shortcut.
 *
 * Both modifiers open it wherever the app runs; only the label changes, and a
 * label naming a key the keyboard does not have is worse than none.
 */
const FIND_KEY = /mac/i.test(navigator.platform || navigator.userAgent) ? "⌘K" : "Ctrl K";

export function Sidebar({
  onNewAgent,
  onEditAgent,
  onNewGroup,
  onEditGroup,
  onOpenSettings,
  onOpenSearch,
  onOpenMenu,
}: Props) {
  const agents = useLiveAgents();
  const groups = useStore((s) => s.groups);
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
      // The one state the operator is the fix for. It reads as an instruction
      // rather than a status because the agent is parked until they act, and
      // the request itself is in a channel they may not have open.
      case "awaitingApproval":
        return { text: "needs you", kind: "asking" };
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

  /** One agent row. Shared by the flat and grouped layouts. */
  const row = (agent: AgentCard) => {
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
        onContextMenu={(event) => {
          event.preventDefault();
          onOpenMenu(agent, { x: event.clientX, y: event.clientY });
        }}
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
  };

  // Pinned agents are lifted out of their group rather than drawn twice. Two
  // rows for one agent would be two nodes in `rowRefs`, and the wire would
  // have to pick one of them to throw a message at.
  //
  // Ordered by when they were made, which is the one order that does not move.
  // The rest of the rail floats whoever just spoke to the top, and a row
  // pinned so it could be found in one glance must not do that.
  const pinned = agents.filter((a) => a.pinned).sort((a, b) => a.createdAt - b.createdAt);

  return (
    <nav className="rail" aria-label="Agents">
      <div className="rail__brand" data-tauri-drag-region>
        <span className="rail__wordmark">Guaca</span>
      </div>

      {/* Looks like a field and behaves like a button, because the field it
          opens onto is the one that does the searching. Two inputs would mean
          deciding which of them holds the query. */}
      <button type="button" className="rail__search" onClick={onOpenSearch}>
        <span aria-hidden="true" className="rail__glass">
          ⌕
        </span>
        <span className="rail__search-label">Search</span>
        <kbd className="rail__key">{FIND_KEY}</kbd>
      </button>

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

      {/* The wire lives on this wrapper rather than on the scroll container, so
          it runs the full height of the rail down to the footer instead of
          stopping wherever the list happens to end. It is only drawn while a
          message is on it; see the rule in styles.css. */}
      <div className="rail__body" data-live={inFlight.length > 0 ? "true" : undefined}>
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

          {/* Whoever the operator wants to hand, above the groups and in the
              same place every time. Unlike the rest of the rail this does not
              reorder itself as agents talk: a row you pinned so you could find
              it must be where you left it. */}
          {pinned.length > 0 && (
            <div className="rail__group">
              <div className="rail__group-head">
                <span className="rail__group-name">Pinned</span>
              </div>
              {pinned.map(row)}
            </div>
          )}

          {/* Every group gets a header, including the only one, because the
              gear on it is where that group's model and endpoint live. */}
          {groups.map((group) => {
            const members = agents.filter((a) => a.groupId === group.id);
            // Drawn here minus whoever is pinned above, but counted in full: a
            // pinned agent is still in this group, still costs it money and is
            // still someone its peers can message.
            const here = members.filter((a) => !a.pinned);
            return (
              <div key={group.id} className="rail__group">
                <div className="rail__group-head">
                  <span className="rail__group-name">{group.name}</span>
                  <span className="rail__group-tail">
                    <TokenMeter groupId={group.id} />
                    <span className="rail__group-count">{members.length}</span>
                    <button
                      type="button"
                      className="rail__gear"
                      onClick={() => onEditGroup(group)}
                      title={`${group.name} settings`}
                      aria-label={`${group.name} settings`}
                    >
                      ⚙
                    </button>
                  </span>
                </div>
                {here.map(row)}
                {members.length === 0 && <p className="rail__empty">No agents in here.</p>}
              </div>
            );
          })}

          {/* Anything whose group did not come back still gets drawn. The rail
              hiding an agent is worse than the rail looking untidy, and an
              empty group list used to hide every agent at once. */}
          {agents.filter((a) => !a.pinned && !groups.some((g) => g.id === a.groupId)).map(row)}

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
        <button type="button" className="btn btn--rail" onClick={onNewGroup}>
          <span aria-hidden="true" className="rail__hash">
            +
          </span>
          New group
        </button>
        <button type="button" className="btn btn--rail" onClick={onOpenSettings}>
          <span aria-hidden="true" className="rail__hash">
            ⚙
          </span>
          App settings
        </button>
      </div>
    </nav>
  );
}
