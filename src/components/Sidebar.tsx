import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { AgentAvatar, type Look } from "../avatars/AgentAvatar";
import { FLIGHT_MS, roleOf, usePulseChoreography } from "../lib/choreography";
import { prefersReducedMotion } from "../lib/motion";
import { type DropTarget, railOrder } from "../lib/rail";
import { ACTIVITY_CHANNEL, useLiveAgents, useStore } from "../lib/store";
import { relativeTime, useNow } from "../lib/time";
import type { Activity, AgentCard, AgentId, Group } from "../lib/types";
import { GroupOrb } from "./GroupOrb";
import { TokenMeter } from "./TokenMeter";

interface Props {
  onNewAgent: () => void;
  onEditAgent: (agent: AgentCard) => void;
  onNewGroup: () => void;
  onEditGroup: (group: Group) => void;
  onOpenCafeteria: () => void;
  onOpenSettings: () => void;
  onOpenSearch: () => void;
  /** Where the operator right-clicked, and on whom. */
  onOpenMenu: (agent: AgentCard, at: { x: number; y: number }) => void;
}

/**
 * How this machine writes the find shortcut.
 *
 * Both modifiers open it wherever the app runs; only the label changes, and a
 * label naming a key the keyboard does not have is worse than none.
 */
const FIND_KEY = /mac/i.test(navigator.platform || navigator.userAgent) ? "⌘K" : "Ctrl K";

/**
 * How far the pointer travels before a press becomes a drag.
 *
 * A row is a button first. Anything smaller than this and selecting an agent
 * with a hand that is not perfectly still starts rearranging the rail instead.
 */
const DRAG_SLOP = 5;

/** How close to an edge of the list the pointer scrolls it, and by how much. */
const EDGE = 36;
const EDGE_STEP = 14;

/** A row in flight, and what it is currently over. */
interface Drag {
  id: AgentId;
  over: DropTarget | null;
}

export function Sidebar({
  onNewAgent,
  onEditAgent,
  onNewGroup,
  onEditGroup,
  onOpenCafeteria,
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
  const railGroup = useStore((s) => s.railGroup);
  const focusGroup = useStore((s) => s.focusGroup);
  const dropAgent = useStore((s) => s.dropAgent);
  const now = useNow();

  const listRef = useRef<HTMLDivElement>(null);
  const rowRefs = useRef(new Map<AgentId, HTMLButtonElement>());
  const [rowCenters, setRowCenters] = useState<Map<AgentId, number>>(new Map());
  const previousTops = useRef(new Map<AgentId, number>());

  /** The press that has not yet travelled far enough to be a drag. */
  const press = useRef<{ id: AgentId; x: number; y: number } | null>(null);
  const [drag, setDrag] = useState<Drag | null>(null);
  /**
   * Where the pointer is, and the thing that follows it.
   *
   * A ref and a direct style write rather than state, because this changes on
   * every pointer frame and every row in the rail has an animating character in
   * it. Rendering the whole rail sixty times a second to move one box is the
   * one thing that would make dragging feel worse than not being able to.
   */
  const point = useRef({ x: 0, y: 0 });
  const heldRef = useRef<HTMLDivElement>(null);

  // Messages arrive in bursts; this spaces them out so each throw is watchable.
  const { staged, inFlight } = usePulseChoreography(pulses, dismissPulse);

  const focused = groups.find((g) => g.id === railGroup) ?? null;

  /**
   * How every section of the rail is ordered right now.
   *
   * Frozen while something is being dragged, because dragging is arranging: a
   * row dropped below a peer that is only near the top because it happens to be
   * mid-turn would land somewhere the operator never aimed at, and the rail
   * would look like it had ignored the gesture the moment that turn ended.
   */
  const shape = { activity, lastActive, frozen: drag !== null };
  const dragging = drag?.id ?? null;

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
    // Everything the drawn order is computed from. Activity and recency are in
    // here because a row lifted for working moves without the roster changing,
    // and a move nothing slid is a row that jumped.
  }, [agents, activity, lastActive, dragging, railGroup]);

  /** Moves the thing in hand to where the pointer is, without a render. */
  const place = useCallback(() => {
    const node = heldRef.current;
    if (!node) return;
    node.style.left = `${point.current.x}px`;
    node.style.top = `${point.current.y}px`;
  }, []);

  // The first frame of a drag: the box has only just been put in the tree, so
  // the handler that has been tracking the pointer had nothing to move.
  useLayoutEffect(place, [place, drag?.id]);

  /**
   * The rest of a drag, once one has started.
   *
   * On the window rather than on the rows, because the pointer leaves the row it
   * picked up almost immediately and a release outside the rail still has to end
   * the drag. Pointer events rather than HTML5 drag and drop: `dragDropEnabled`
   * is what lets a dropped document reach Rust without entering the renderer,
   * and it is the same setting that stops `dragstart` firing inside the webview
   * on some platforms. A rail that only rearranges on macOS is not a feature.
   */
  useEffect(() => {
    const move = (event: PointerEvent) => {
      const held = press.current;
      if (held && !drag) {
        if (Math.abs(event.clientX - held.x) + Math.abs(event.clientY - held.y) < DRAG_SLOP) return;
        // Whatever the press began selecting is not what the operator meant.
        window.getSelection()?.removeAllRanges();
        // Released here, not on the way out: this handler runs again on the
        // next movement with a closure that still thinks nothing is being
        // dragged, and a press it could still see would start a second drag on
        // top of this one and lose whatever the pointer had reached.
        press.current = null;
        point.current = { x: event.clientX, y: event.clientY };
        setDrag({ id: held.id, over: null });
        return;
      }
      if (!drag) return;

      point.current = { x: event.clientX, y: event.clientY };
      place();

      // Reaching a row that is off the bottom of the rail has to be possible
      // without letting go. Stepped per movement rather than on a timer: a
      // pointer held still in the margin is a pointer that has arrived.
      const list = listRef.current;
      if (!list || list.scrollHeight <= list.clientHeight) return;
      const box = list.getBoundingClientRect();
      if (event.clientY < box.top + EDGE) list.scrollBy({ top: -EDGE_STEP });
      else if (event.clientY > box.bottom - EDGE) list.scrollBy({ top: EDGE_STEP });
    };

    const release = () => {
      const finished = drag;
      press.current = null;
      setDrag(null);
      if (finished?.over) void dropAgent(finished.id, finished.over);
    };

    const cancel = () => {
      press.current = null;
      setDrag(null);
    };

    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") cancel();
    };

    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", release);
    window.addEventListener("pointercancel", cancel);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", release);
      window.removeEventListener("pointercancel", cancel);
      window.removeEventListener("keydown", onKey);
    };
    // Rebound when the drag starts, ends, or reaches a different target: a
    // handful of times per drag. The pointer's own position is deliberately not
    // in here, or this would be four listeners torn down and replaced on every
    // frame of every drag.
  }, [drag, dropAgent, place]);

  /** Marks what the pointer is over, while it is over it. */
  const hover = useCallback((target: DropTarget | null) => {
    setDrag((current) => {
      if (!current) return current;
      if (target === null) return { ...current, over: null };
      return { ...current, over: target };
    });
  }, []);

  /** Whether a drop target is the one currently under the pointer. */
  const isOver = (target: DropTarget): boolean => {
    const over = drag?.over;
    if (!over || over.kind !== target.kind) return false;
    return over.kind === "pinned" || ("id" in over && "id" in target && over.id === target.id);
  };

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

  /** One agent row. Shared by every section and both views. */
  const row = (agent: AgentCard) => {
    const role = roleOf(staged, agent.id);
    const label = meta(agent.id, activity[agent.id]);
    const target: DropTarget = { kind: "row", id: agent.id };

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
        data-held={dragging === agent.id ? "true" : undefined}
        data-over={dragging && dragging !== agent.id && isOver(target) ? "true" : undefined}
        style={{ "--accent": agent.color } as React.CSSProperties}
        onClick={() => void select(agent.id)}
        onDoubleClick={() => onEditAgent(agent)}
        onContextMenu={(event) => {
          event.preventDefault();
          onOpenMenu(agent, { x: event.clientX, y: event.clientY });
        }}
        // The press is remembered and nothing else happens yet: a row is a
        // button first, and it only becomes a handle once the pointer moves.
        onPointerDown={(event) => {
          if (event.button !== 0) return;
          press.current = { id: agent.id, x: event.clientX, y: event.clientY };
        }}
        onPointerEnter={() => hover(target)}
        onPointerLeave={() => hover(null)}
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

  /** The controls that belong to a group, wherever its name is drawn. */
  const groupTail = (group: Group, count: number) => (
    <span className="rail__group-tail">
      <TokenMeter groupId={group.id} />
      <span className="rail__group-count">{count}</span>
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
  );

  // Pinned agents are lifted out of their group rather than drawn twice. Two
  // rows for one agent would be two nodes in `rowRefs`, and the wire would
  // have to pick one of them to throw a message at.
  //
  // Ordered by the arrangement, which is the one order that does not move on
  // its own. The rest of a section lifts whoever is working to the top, and a
  // row pinned so it could be found in one glance must not do that.
  const pinned = railOrder(
    agents.filter((a) => a.pinned),
    shape,
  );

  const held = drag ? agents.find((a) => a.id === drag.id) : undefined;

  return (
    <nav className="rail" aria-label="Agents" data-dragging={drag ? "true" : undefined}>
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

      {/* The crews, as things you can go inside and drop somebody into. Absent
          while there is one group, which is the state most workspaces are in:
          a strip offering a choice of one is a row of chrome that never changes
          and a drop target that cannot move anybody anywhere. */}
      {groups.length > 1 && (
        <nav className="rail__strip" aria-label="Groups">
          <button
            type="button"
            className="orb orb--all"
            aria-current={focused === null}
            aria-label={`All groups, ${agents.length} agents`}
            onClick={() => void focusGroup(null)}
          >
            <span className="orb__ring">
              {/* The count, because the word is already under it and a circle
                  that says "all" above a label that says "All" says one thing
                  twice. A group's name can be anything, including "everyone",
                  so this one must not be a name at all. */}
              <span className="orb__all-count">{agents.length}</span>
            </span>
            <span className="orb__name">All</span>
          </button>

          {groups.map((group) => (
            <GroupOrb
              key={group.id}
              group={group}
              members={agents.filter((a) => a.groupId === group.id)}
              activity={activity}
              current={focused?.id === group.id}
              over={isOver({ kind: "group", id: group.id })}
              onOpen={() => void focusGroup(focused?.id === group.id ? null : group.id)}
              onDragOver={() => hover({ kind: "group", id: group.id })}
              onDragOut={() => hover(null)}
            />
          ))}
        </nav>
      )}

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

          {focused ? (
            // Inside one group. The heading carries the name and the controls
            // that were squeezed onto a 0.6rem label in the overview, because
            // here there is one group on screen and room to say so. The pins
            // stay at the head of the list rather than in a section of their
            // own: everybody drawn here is in this crew already, so a second
            // heading over one or two rows would divide nothing.
            <div
              className="rail__group rail__group--open"
              data-over={isOver({ kind: "group", id: focused.id }) ? "true" : undefined}
              onPointerEnter={() => hover({ kind: "group", id: focused.id })}
              onPointerLeave={() => hover(null)}
            >
              <div className="rail__open-head">
                <span className="rail__open-name">{focused.name}</span>
                {groupTail(focused, agents.filter((a) => a.groupId === focused.id).length)}
              </div>
              {railOrder(
                agents.filter((a) => a.groupId === focused.id),
                { ...shape, pinnedFirst: true },
              ).map(row)}
              {agents.every((a) => a.groupId !== focused.id) && (
                <p className="rail__empty">
                  Nobody is in here yet. Drag an agent onto this group, or make one.
                </p>
              )}
            </div>
          ) : (
            <>
              {/* Whoever the operator wants to hand, above the groups and in
                  the same place every time. */}
              {pinned.length > 0 && (
                <div
                  className="rail__group"
                  data-over={isOver({ kind: "pinned" }) ? "true" : undefined}
                  onPointerEnter={() => hover({ kind: "pinned" })}
                  onPointerLeave={() => hover(null)}
                >
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
                // Drawn here minus whoever is pinned above, but counted in
                // full: a pinned agent is still in this group, still costs it
                // money and is still someone its peers can message.
                const here = railOrder(
                  members.filter((a) => !a.pinned),
                  shape,
                );
                return (
                  // The whole block catches a drop, not just the heading:
                  // anywhere in a group that is not a row means the group and no
                  // particular place in it, which is all an empty one can offer.
                  <div
                    key={group.id}
                    className="rail__group"
                    data-over={isOver({ kind: "group", id: group.id }) ? "true" : undefined}
                    onPointerEnter={() => hover({ kind: "group", id: group.id })}
                    onPointerLeave={() => hover(null)}
                  >
                    <div className="rail__group-head">
                      <span className="rail__group-name">{group.name}</span>
                      {groupTail(group, members.length)}
                    </div>
                    {here.map(row)}
                    {members.length === 0 && <p className="rail__empty">No agents in here.</p>}
                  </div>
                );
              })}

              {/* Anything whose group did not come back still gets drawn. The
                  rail hiding an agent is worse than the rail looking untidy,
                  and an empty group list used to hide every agent at once. */}
              {railOrder(
                agents.filter((a) => !a.pinned && !groups.some((g) => g.id === a.groupId)),
                shape,
              ).map(row)}
            </>
          )}

          {agents.length === 0 && <p className="rail__empty">No agents yet.</p>}
        </div>
      </div>

      <div className="rail__foot">
        {/* Above "New agent" because it is the faster of the two ways to add
            somebody, and the one an operator reaches for more than once. */}
        <button type="button" className="btn btn--rail" onClick={onOpenCafeteria}>
          <span aria-hidden="true" className="rail__hash">
            ☰
          </span>
          Cafeteria
        </button>
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

      {/* What the hand is holding. Drawn at the pointer and outside the list so
          nothing it passes over can clip it, and transparent to the pointer so
          the row underneath is still the row being aimed at. */}
      {drag && held && (
        <div className="rail__held" aria-hidden="true" ref={heldRef}>
          <AgentAvatar
            avatar={held.avatar}
            color={held.color}
            size="sm"
            seed={held.id}
            lifecycle={held.lifecycle}
          />
          <span className="rail__held-name">{held.name}</span>
        </div>
      )}
    </nav>
  );
}
