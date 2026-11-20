import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AgentAvatar, type Look } from "../avatars/AgentAvatar";
import { FLIGHT_MS, roleOf, usePulseChoreography } from "../lib/choreography";
import { useDismiss } from "../lib/dismiss";
import { doing } from "../lib/doing";
import { useForYouCount, useShown } from "../lib/held";
import { FIND_KEY, IS_MAC } from "../lib/keybinds";
import { prefersReducedMotion } from "../lib/motion";
import { newestByAgent } from "../lib/outcomes";
import { questions } from "../lib/questions";
import { type DropTarget, railOrder } from "../lib/rail";
import { useLiveAgents, useStore } from "../lib/store";
import { useNow } from "../lib/time";
import { desktop } from "../lib/transport";
import type { AgentCard, AgentId, Group, GroupId } from "../lib/types";
import { Brand } from "./Brand";
import { GroupRail } from "./GroupRail";
import { Icon } from "./Icon";
import { NewMenu } from "./NewMenu";
import { SpendTag, useSpendTag } from "./Spend";

const LIGHTS = desktop && IS_MAC;

interface Props {
  onOpenChannel?: () => void;
  onEditAgent: (agent: AgentCard) => void;
  onEditGroup: (group: Group) => void;
  onOpenCafeteria: () => void;
  onOpenCalendar: () => void;
  onOpenArtifacts: () => void;
  onOpenSettings: () => void;
  onOpenSearch: () => void;
  onNewAgent: () => void;
  onNewGroup: () => void;
  onOpenMenu: (agent: AgentCard, at: { x: number; y: number }) => void;
}

const DRAG_SLOP = 5;

const EDGE = 36;
const EDGE_STEP = 14;

type Held = { kind: "agent"; id: AgentId } | { kind: "group"; id: GroupId };

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
  const { activity, stuck, pending } = useShown();
  const showForYou = useStore((s) => s.showForYou);
  const trail = useStore((s) => s.trail);
  const finishedAt = useStore((s) => s.finishedAt);
  const lastActive = useStore((s) => s.lastActive);
  const outcomes = useStore((s) => s.outcomes);
  const finished = useMemo(() => newestByAgent(outcomes), [outcomes]);
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
  const crewsRef = useRef<HTMLDivElement>(null);
  const rowRefs = useRef(new Map<AgentId, HTMLButtonElement>());
  const [rowCenters, setRowCenters] = useState<Map<AgentId, number>>(new Map());
  const previousTops = useRef(new Map<AgentId, number>());

  const press = useRef<{ held: Held; x: number; y: number } | null>(null);
  const [pressed, setPressed] = useState(false);
  const [drag, setDrag] = useState<Drag | null>(null);
  const point = useRef({ x: 0, y: 0 });
  const heldRef = useRef<HTMLDivElement>(null);

  const { staged, inFlight } = usePulseChoreography(pulses, dismissPulse);

  const spend = useSpendTag();

  const focused = groups.find((g) => g.id === railGroup) ?? null;

  const dragging = drag?.held.kind === "agent" ? drag.held.id : null;
  const shape = { activity, lastActive, frozen: dragging !== null };

  useLayoutEffect(() => {
    const measure = () => {
      if (!listRef.current) return;
      const centers = new Map<AgentId, number>();
      for (const [id, node] of rowRefs.current) {
        centers.set(id, node.offsetTop + node.offsetHeight / 2);
      }
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
  }, [agents, activity, lastActive, dragging, railGroup]);

  const place = useCallback(() => {
    const node = heldRef.current;
    if (!node) return;
    node.style.left = `${point.current.x}px`;
    node.style.top = `${point.current.y}px`;
  }, []);

  useLayoutEffect(place, [place, drag?.held.id]);

  useDismiss(
    () => {
      press.current = null;
      setPressed(false);
      setDrag(null);
    },
    pressed || drag !== null,
  );

  useEffect(() => {
    const move = (event: PointerEvent) => {
      const held = press.current;
      if (held && !drag) {
        if (Math.abs(event.clientX - held.x) + Math.abs(event.clientY - held.y) < DRAG_SLOP) return;
        window.getSelection()?.removeAllRanges();
        press.current = null;
        setPressed(false);
        point.current = { x: event.clientX, y: event.clientY };
        setDrag({ held: held.held, over: null });
        return;
      }
      if (!drag) return;

      point.current = { x: event.clientX, y: event.clientY };
      place();

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
      setPressed(false);
      setDrag(null);
      if (!finished?.over) return;
      if (finished.held.kind === "agent") void dropAgent(finished.held.id, finished.over);
      else if (finished.over.kind === "group") void dropGroup(finished.held.id, finished.over.id);
    };

    const cancel = () => {
      press.current = null;
      setPressed(false);
      setDrag(null);
    };

    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", release);
    window.addEventListener("pointercancel", cancel);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", release);
      window.removeEventListener("pointercancel", cancel);
    };
  }, [drag, dropAgent, dropGroup, place]);

  const grab = (held: Held, event: React.PointerEvent) => {
    if (event.button !== 0 || event.pointerType === "touch") return;
    press.current = { held, x: event.clientX, y: event.clientY };
    setPressed(true);
  };

  const aim = useCallback((target: DropTarget | null, circle: boolean) => {
    setDrag((current) => {
      if (!current) return current;
      if (current.held.kind === "group" && !circle) return current;
      return { ...current, over: target };
    });
  }, []);

  const hover = (target: DropTarget | null) => aim(target, false);

  const isOver = (target: DropTarget): boolean => {
    const over = drag?.over;
    if (!over || over.kind !== target.kind) return false;
    return over.id === target.id;
  };

  const sectionOver = (id: GroupId): boolean => dragging !== null && isOver({ kind: "group", id });

  const facing = (self: AgentId, other: AgentId | undefined): Look => {
    if (!other) return null;
    const mine = rowCenters.get(self);
    const theirs = rowCenters.get(other);
    if (mine === undefined || theirs === undefined) return null;
    return theirs > mine ? "down" : "up";
  };

  const row = (agent: AgentCard) => {
    const role = roleOf(staged, agent.id);
    const label = doing(agent.id, { activity, stuck, trail, lastActive, finished }, now);
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
        title={label.title}
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
          size="xs"
          gesture={role.gesture}
          look={facing(agent.id, role.facing ?? undefined)}
          says={role.says}
        />
        <span className="agent-row__title">
          <span className="agent-row__name">{agent.name}</span>
          {agent.pinned && (
            <span className="agent-row__pin" role="img" aria-label="pinned">
              <Icon name="pin" size={16} />
            </span>
          )}
        </span>
        <span className="agent-row__meta" data-state={label.kind}>
          {label.text}
        </span>
      </button>
    );
  };

  const gear = (group: Group) => (
    <button
      type="button"
      className="rail__gear"
      onClick={() => onEditGroup(group)}
      title={`${group.name} settings`}
      aria-label={`${group.name} settings`}
    >
      <Icon name="gear" size={16} />
    </button>
  );

  const inHand = drag?.held;
  const heldAgent = inHand?.kind === "agent" ? agents.find((a) => a.id === inHand.id) : undefined;

  const waiting = useForYouCount();

  return (
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

      {spend.shown && !drag && <SpendTag groupId={spend.shown.id} at={spend.shown} />}

      <nav className="rail" aria-label="Agents" data-dragging={drag ? "true" : undefined}>
        <div className="rail__brand" data-tauri-drag-region data-lights={LIGHTS ? "" : undefined}>
          <Brand waiting={pending.length > 0} />
          <NewMenu
            onNewAgent={onNewAgent}
            onNewGroup={onNewGroup}
            onOpenCafeteria={onOpenCafeteria}
          />
        </div>

        <label className="mobile-crews field">
          <span className="field__label">Group</span>
          <select
            className="input"
            value={railGroup ?? ""}
            onChange={(event) => void focusGroup(event.target.value || null)}
          >
            <option value="">All groups</option>
            {groups.map((group) => (
              <option key={group.id} value={group.id}>
                {group.name}
              </option>
            ))}
          </select>
        </label>

        <button type="button" className="rail__search" onClick={onOpenSearch}>
          <span aria-hidden="true" className="rail__glass">
            <Icon name="search" size={16} />
          </span>
          <span className="rail__search-label">Search</span>
          <kbd className="rail__key">{FIND_KEY}</kbd>
        </button>

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
                {groups.map((group) => {
                  const members = agents.filter((a) => a.groupId === group.id);
                  const here = railOrder(members, shape);
                  return (
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
                        <span className="rail__group-count">{here.length}</span>
                        {gear(group)}
                      </div>
                      {here.map(row)}
                      {members.length === 0 && agents.length > 0 && (
                        <p className="rail__empty">No agents in here.</p>
                      )}
                    </div>
                  );
                })}

                {railOrder(
                  agents.filter((a) => !groups.some((g) => g.id === a.groupId)),
                  shape,
                ).map(row)}
              </>
            )}

            {agents.length === 0 && <p className="rail__empty">No agents yet.</p>}
          </div>
        </div>

        <div className="rail__foot">
          <button
            type="button"
            className="rail__place"
            aria-label={waiting > 0 ? `For you, ${questions(waiting)}` : "For you"}
            onClick={() => showForYou(true)}
          >
            <span>For you</span>
            <b data-waiting={waiting > 0 ? "true" : undefined}>{waiting}</b>
          </button>
          <button type="button" className="rail__place" onClick={onOpenCalendar}>
            <span>Calendar</span>
            <b>{new Date(now).toLocaleDateString([], { weekday: "short" })}</b>
          </button>
          <button type="button" className="rail__place" onClick={onOpenArtifacts}>
            <span>Artifacts</span>
          </button>
          <button
            type="button"
            className="rail__place"
            onClick={onOpenSettings}
            aria-label="App settings"
            title="App settings"
          >
            <span>Settings</span>
          </button>
        </div>

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
