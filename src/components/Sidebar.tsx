import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";

import { AgentAvatar, type Look } from "../avatars/AgentAvatar";
import { FLIGHT_MS, roleOf, usePulseChoreography } from "../lib/choreography";
import { doing } from "../lib/doing";
import { IS_MAC } from "../lib/keybinds";
import { prefersReducedMotion } from "../lib/motion";
import { type DropTarget, railOrder } from "../lib/rail";
import { useLiveAgents, useStore } from "../lib/store";
import { useNow } from "../lib/time";
import { desktop } from "../lib/transport";
import type { AgentCard, AgentId, Group, GroupId } from "../lib/types";
import { ArtifactMark } from "./Artifacts";
import { Brand } from "./Brand";
import { GroupRail } from "./GroupRail";
import { NewMenu } from "./NewMenu";
import { SpendTag, useSpendTag } from "./Spend";

/** Only a Mac window draws its own buttons over the rail's top left corner. */
const LIGHTS = desktop && IS_MAC;

interface Props {
  onOpenChannel?: () => void;
  onEditAgent: (agent: AgentCard) => void;
  onEditGroup: (group: Group) => void;
  onOpenCafeteria: () => void;
  /** The workspace calendar: every crew's dates, in one place. */
  onOpenCalendar: () => void;
  /** The pages the crews keep, scoped to whichever crew the rail is inside. */
  onOpenArtifacts: () => void;
  onOpenSettings: () => void;
  onOpenSearch: () => void;
  /** The plus in the title strip, beside the pair. App-level, not any one row's. */
  onNewAgent: () => void;
  onNewGroup: () => void;
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

/**
 * What a press picked up: an agent's row, or a crew's circle.
 *
 * One drag for both, because they share every part of the gesture but where it
 * may land. A row lands on a row or a crew; a circle lands among the circles
 * and nowhere else, since the rail's rows and sections are places for agents.
 */
type Held = { kind: "agent"; id: AgentId } | { kind: "group"; id: GroupId };

/** Something in flight, and what it is currently over. */
interface Drag {
  held: Held;
  over: DropTarget | null;
}

export function Sidebar({
  onOpenChannel,
  onEditAgent,
  onEditGroup,
  onOpenCafeteria,
  onOpenCalendar,
  onOpenArtifacts,
  onOpenSettings,
  onOpenSearch,
  onNewAgent,
  onNewGroup,
  onOpenMenu,
}: Props) {
  const agents = useLiveAgents();
  const groups = useStore((s) => s.groups);
  const building = useStore((s) => s.building);
  const activity = useStore((s) => s.activity);
  const stuck = useStore((s) => s.stuck);
  const decisions = useStore((s) => s.decisions);
  const pending = useStore((s) => s.pending);
  const showForYou = useStore((s) => s.showForYou);
  const decisionCount = decisions.filter(
    (item) => item.status === "pending" || (item.status === "answered" && item.interrupted),
  ).length;
  /* The rail is the one surface that draws a whole crew at once, so it is the
     one that pays for the signals the quieter moods are read from. */
  const trail = useStore((s) => s.trail);
  const finishedAt = useStore((s) => s.finishedAt);
  const lastActive = useStore((s) => s.lastActive);
  const selected = useStore((s) => s.selected);
  const select = useStore((s) => s.select);
  const pulses = useStore((s) => s.pulses);
  const dismissPulse = useStore((s) => s.dismissPulse);
  const railGroup = useStore((s) => s.railGroup);
  const focusGroup = useStore((s) => s.focusGroup);
  const dropAgent = useStore((s) => s.dropAgent);
  const dropGroup = useStore((s) => s.dropGroup);
  const now = useNow();

  const listRef = useRef<HTMLDivElement>(null);
  /** The crews' own list, which scrolls under a drag the way the rail's does. */
  const crewsRef = useRef<HTMLDivElement>(null);
  const rowRefs = useRef(new Map<AgentId, HTMLButtonElement>());
  const [rowCenters, setRowCenters] = useState<Map<AgentId, number>>(new Map());
  const previousTops = useRef(new Map<AgentId, number>());

  /** The press that has not yet traveled far enough to be a drag. */
  const press = useRef<{ held: Held; x: number; y: number } | null>(null);
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

  /** Which crew's heading is being asked what it has spent. One per pointer. */
  const spend = useSpendTag();

  const focused = groups.find((g) => g.id === railGroup) ?? null;

  /**
   * How every section of the rail is ordered right now.
   *
   * Frozen while a row is being dragged, because dragging is arranging: a row
   * dropped below a peer that is only near the top because it happens to be
   * mid-turn would land somewhere the operator never aimed at, and the rail
   * would look like it had ignored the gesture the moment that turn ended. A
   * crew in hand arranges nothing in the rail, so the rail is left as it was.
   */
  const dragging = drag?.held.kind === "agent" ? drag.held.id : null;
  const shape = { activity, lastActive, frozen: dragging !== null };

  // One layout pass does two jobs: slide rows that moved, and record where
  // every row ended up so the traveling message knows where to fly.
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
  useLayoutEffect(place, [place, drag?.held.id]);

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
        setDrag({ held: held.held, over: null });
        return;
      }
      if (!drag) return;

      point.current = { x: event.clientX, y: event.clientY };
      place();

      // Reaching a row that is off the bottom of the rail, or a crew off the
      // bottom of the column, has to be possible without letting go. Stepped
      // per movement rather than on a timer: a pointer held still in the
      // margin is a pointer that has arrived. The column when the pointer is
      // over it, which it can only be while it is out, since it is drawn over
      // the rail; the rail otherwise, and only for a row, which is the one
      // thing that can land there.
      const crews = crewsRef.current;
      const column = crews?.getBoundingClientRect();
      const overCrews = column && event.clientX >= column.left && event.clientX <= column.right;
      const list = overCrews ? crews : drag.held.kind === "agent" ? listRef.current : null;
      if (!list || list.scrollHeight <= list.clientHeight) return;
      const box = list.getBoundingClientRect();
      if (event.clientY < box.top + EDGE) list.scrollBy({ top: -EDGE_STEP });
      else if (event.clientY > box.bottom - EDGE) list.scrollBy({ top: EDGE_STEP });
    };

    const release = () => {
      const finished = drag;
      press.current = null;
      setDrag(null);
      if (!finished?.over) return;
      if (finished.held.kind === "agent") void dropAgent(finished.held.id, finished.over);
      else if (finished.over.kind === "group") void dropGroup(finished.held.id, finished.over.id);
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
  }, [drag, dropAgent, dropGroup, place]);

  /**
   * A press, remembered and nothing else yet. A row and a circle are both
   * buttons first, and only become handles once the pointer moves.
   */
  const grab = (held: Held, event: React.PointerEvent) => {
    if (event.button !== 0 || event.pointerType === "touch") return;
    press.current = { held, x: event.clientX, y: event.clientY };
  };

  /**
   * Marks what the pointer is over, while it is over it.
   *
   * `circle` says the target is a crew's circle rather than a row or a section
   * of the rail. A circle catches anything; the rail catches an agent and
   * nothing else, so a crew carried across it is aimed at nothing and a release
   * there moves nothing.
   */
  const aim = useCallback((target: DropTarget | null, circle: boolean) => {
    setDrag((current) => {
      if (!current) return current;
      if (current.held.kind === "group" && !circle) return current;
      return { ...current, over: target };
    });
  }, []);

  const hover = (target: DropTarget | null) => aim(target, false);

  /** Whether a drop target is the one currently under the pointer. */
  const isOver = (target: DropTarget): boolean => {
    const over = drag?.over;
    if (!over || over.kind !== target.kind) return false;
    return over.id === target.id;
  };

  /**
   * Whether a crew's section of the rail is where a dragged agent would land.
   * An agent only: a crew over a circle is aimed at the same target a section
   * is, and the section lit under it said the crew was going into itself.
   */
  const sectionOver = (id: GroupId): boolean => dragging !== null && isOver({ kind: "group", id });

  /** Which way an agent should turn to face a peer. */
  const facing = (self: AgentId, other: AgentId | undefined): Look => {
    if (!other) return null;
    const mine = rowCenters.get(self);
    const theirs = rowCenters.get(other);
    if (mine === undefined || theirs === undefined) return null;
    return theirs > mine ? "down" : "up";
  };

  /** One agent row. Shared by every section and both views. */
  const row = (agent: AgentCard) => {
    const role = roleOf(staged, agent.id);
    const label = doing(agent.id, { activity, stuck, building, trail, lastActive }, now);
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
        onClick={() => {
          void select(agent.id);
          onOpenChannel?.();
        }}
        onDoubleClick={() => onEditAgent(agent)}
        onContextMenu={(event) => {
          event.preventDefault();
          onOpenMenu(agent, { x: event.clientX, y: event.clientY });
        }}
        onPointerDown={(event) => grab({ kind: "agent", id: agent.id }, event)}
        onPointerEnter={() => hover(target)}
        onPointerLeave={() => hover(null)}
      >
        <AgentAvatar
          avatar={agent.avatar}
          color={agent.color}
          activity={activity[agent.id]}
          lifecycle={agent.lifecycle}
          work={trail[agent.id]}
          escalated={stuck.some((one) => one.agentId === agent.id)}
          finishedAt={finishedAt[agent.id]}
          seed={agent.id}
          gesture={role.gesture}
          look={facing(agent.id, role.facing ?? undefined)}
          says={role.says}
        />
        <span className="agent-row__title">
          <span className="agent-row__name">{agent.name}</span>
          {/* The only thing on screen that says a row is pinned rather than
              first. Being at the head of a crew is what a pin does, and a crew
              whose pinned member is also the one the operator arranged at the
              top looks exactly like a pin that did nothing. */}
          {agent.pinned && (
            <svg className="agent-row__pin" viewBox="0 0 12 12" role="img" aria-label="pinned">
              <path
                d="M6 1.4a2.6 2.6 0 0 1 .9 5.04L6 10.6l-.9-4.16A2.6 2.6 0 0 1 6 1.4z"
                fill="currentColor"
              />
            </svg>
          )}
        </span>
        <span className="agent-row__meta" data-state={label.kind}>
          {label.text}
        </span>
      </button>
    );
  };

  /**
   * The way into a group's settings, and the only thing left beside its name.
   *
   * The member count that used to hang here is gone: the crews' column says how
   * many are in a crew twice already, since the faces on a circle are seated by
   * how many there are and the tag under the pointer says the number in words.
   * What a crew has spent is gone from the line too, and is what hovering the
   * heading says. Both were readouts of fixed width on a line fifteen rem wide
   * at its widest, and the name was the only thing on it that could give any up.
   */
  const gear = (group: Group) => (
    <button
      type="button"
      className="rail__gear"
      onClick={() => onEditGroup(group)}
      title={`${group.name} settings`}
      aria-label={`${group.name} settings`}
    >
      ⚙
    </button>
  );

  const inHand = drag?.held;
  /** The agent the hand is holding, which the box at the pointer draws. */
  const heldAgent = inHand?.kind === "agent" ? agents.find((a) => a.id === inHand.id) : undefined;

  const waiting = decisionCount + pending.length + stuck.length;

  return (
    // Two columns, one gesture. The crews and the crew are separate surfaces on
    // screen and one drag reaches across both, so they are drawn together
    // rather than made siblings in `App`: a drop onto a circle would otherwise
    // need the whole pointer machinery lifted into a context to be shared with
    // it.
    <>
      <GroupRail
        groups={groups}
        agents={agents}
        activity={activity}
        stuck={stuck}
        focused={railGroup}
        onFocus={(id) => void focusGroup(id)}
        isOver={isOver}
        onDragOver={(target) => aim(target, true)}
        onDragOut={() => aim(null, true)}
        dragging={drag !== null}
        held={inHand?.kind === "group" ? inHand.id : null}
        onPress={(id, event) => grab({ kind: "group", id }, event)}
        listRef={crewsRef}
      />

      {/* Over the rail rather than inside it: the card is fixed to the window
          and the list it hangs off scrolls, and one card for every heading is
          what having one pointer means. Never during a drag, when the rows it
          would cover are the targets being aimed at and the pointer is already
          carrying something. */}
      {spend.shown && !drag && <SpendTag groupId={spend.shown.id} at={spend.shown} />}

      <nav className="rail" aria-label="Agents" data-dragging={drag ? "true" : undefined}>
        {/* The plus rides the drag region rather than sitting under it: a button
            inside one is still a button, and this is the row an operator reads
            first. */}
        <div className="rail__brand" data-tauri-drag-region data-lights={LIGHTS ? "" : undefined}>
          <Brand waiting={pending.length > 0} />
          <NewMenu
            onNewAgent={onNewAgent}
            onNewGroup={onNewGroup}
            onOpenCafeteria={onOpenCafeteria}
          />
        </div>

        <label className="mobile-crews field">
          <span className="field__label">Crew</span>
          <select
            className="input"
            value={railGroup ?? ""}
            onChange={(event) => void focusGroup(event.target.value || null)}
          >
            <option value="">All crews</option>
            {groups.map((group) => (
              <option key={group.id} value={group.id}>
                {group.name}
              </option>
            ))}
          </select>
        </label>

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

        <button type="button" className="rail__for-you" onClick={() => showForYou(true)}>
          <span>For you</span>
          {/* Amber only while something is waiting: a zero in the one color that
            means "answer me" is the app asking about nothing. */}
          <span data-waiting={waiting > 0 ? "true" : undefined}>{waiting}</span>
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

            {focused ? (
              // Inside one group. The heading is the name at reading size, on a
              // line of its own, with what the crew is spending under it: there
              // is one group on screen here, so the name is the heading of the
              // whole column rather than one label among several. The pins head
              // the list rather than sitting in a section of their own:
              // everybody drawn here is in this crew already, so a heading over
              // one or two rows would divide nothing, and the mark on the row is
              // what says which rows those are.
              <div
                className="rail__group rail__group--open"
                data-over={sectionOver(focused.id) ? "true" : undefined}
                onPointerEnter={() => hover({ kind: "group", id: focused.id })}
                onPointerLeave={() => hover(null)}
              >
                <div
                  className="rail__open-head"
                  onPointerEnter={(event) => spend.show(focused.id, event)}
                  onPointerLeave={spend.hide}
                >
                  {/* Both headings ellipse a name that does not fit the rail,
                      and the crew column has no room to say it either, so the
                      full one is on the heading as well as beside the circle.
                      A crew called "Customer research, EMEA" is otherwise two
                      words and a hyphen wherever it is drawn. */}
                  <span className="rail__open-name" title={focused.name}>
                    {focused.name}
                  </span>
                  {gear(focused)}
                </div>
                {railOrder(
                  agents.filter((a) => a.groupId === focused.id),
                  shape,
                ).map(row)}
                {agents.every((a) => a.groupId !== focused.id) && (
                  <p className="rail__empty">
                    Nobody is in here yet. Drag an agent onto this group, or make one.
                  </p>
                )}
              </div>
            ) : (
              <>
                {/* Every group gets a header, including the only one, because the
                  gear on it is where that group's model and endpoint live. A
                  crew's pins are at the head of it and nowhere else. They had a
                  section of their own above the groups, which made a pin the one
                  arrangement that was undone by going inside the crew it was
                  arranging: the list on screen was the group's, and the row the
                  operator had just pinned was in a section that was not being
                  drawn. */}
                {groups.map((group) => {
                  const members = agents.filter((a) => a.groupId === group.id);
                  const here = railOrder(members, shape);
                  return (
                    // The whole block catches a drop, not just the heading:
                    // anywhere in a group that is not a row means the group and no
                    // particular place in it, which is all an empty one can offer.
                    <div
                      key={group.id}
                      className="rail__group"
                      data-over={sectionOver(group.id) ? "true" : undefined}
                      onPointerEnter={() => hover({ kind: "group", id: group.id })}
                      onPointerLeave={() => hover(null)}
                    >
                      <div
                        className="rail__group-head"
                        onPointerEnter={(event) => spend.show(group.id, event)}
                        onPointerLeave={spend.hide}
                      >
                        <span className="rail__group-name" title={group.name}>
                          {group.name}
                        </span>
                        {gear(group)}
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
                  agents.filter((a) => !groups.some((g) => g.id === a.groupId)),
                  shape,
                ).map(row)}
              </>
            )}

            {agents.length === 0 && <p className="rail__empty">No agents yet.</p>}
          </div>
        </div>

        {/* The places the rail does not list, as one row. Four stacked links
            were a fifth of the column's height taken from the agents, and a
            row per place is a footer that gets taller every time the app gains
            one. Settings stays last: a phone hides it, because it has a tab of
            its own there. */}
        <div className="rail__foot">
          <button type="button" className="rail__place" onClick={onOpenCalendar}>
            {/* Today's date, which is the one thing a calendar can say before
                it is opened, and what makes the mark read as one. */}
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <rect x="4" y="5" width="16" height="15.5" rx="2" />
              <path d="M4 9.5h16M8.5 3v4M15.5 3v4" />
              <text x="12" y="17.8" textAnchor="middle" fontSize="8" fontWeight="600">
                {new Date(now).getDate()}
              </text>
            </svg>
            Calendar
          </button>
          <button type="button" className="rail__place" onClick={onOpenArtifacts}>
            <ArtifactMark />
            Artifacts
          </button>
          {/* Named in full for anything that reads the label rather than the
              position: inside a crew, the crew's own settings are one click
              away at the top of the column. */}
          <button
            type="button"
            className="rail__place"
            onClick={onOpenSettings}
            aria-label="App settings"
            title="App settings"
          >
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path d="M4 6h16M4 12h16M4 18h16M8 3v6M16 9v6M10 15v6" />
            </svg>
            Settings
          </button>
        </div>

        {/* What the hand is holding. Drawn at the pointer and outside the list so
          nothing it passes over can clip it, and transparent to the pointer so
          the row underneath is still the row being aimed at. An agent only: a
          crew is carried along a column one circle wide, where the pointer is
          always on the circle it is aimed at and that circle's tag is already
          beside it. A second name there read as two labels for one circle, and
          the circle in hand is still on screen, dimmed where it was. */}
        {heldAgent && (
          <div className="rail__held" aria-hidden="true" ref={heldRef}>
            <AgentAvatar
              avatar={heldAgent.avatar}
              color={heldAgent.color}
              size="sm"
              seed={heldAgent.id}
              lifecycle={heldAgent.lifecycle}
            />
            <span className="rail__held-name">{heldAgent.name}</span>
          </div>
        )}
      </nav>
    </>
  );
}
