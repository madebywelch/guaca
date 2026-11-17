import {
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useRef,
  useState,
} from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import type { Ribbon as Layout } from "../lib/ribbon";
import { useStore } from "../lib/store";
import type { AgentId } from "../lib/types";

interface Props {
  ribbon: Layout;
  onOpenAgent: (agentId: AgentId) => void;
  onOpenPile: () => void;
  openAgentId?: AgentId | null;
  onTrackWidth?: (px: number) => void;
}

const PILE_SHOWN = 5;
const PILE_KEY = "pile";

const at = (x: number) => ({ "--ribbon-x": x }) as CSSProperties;

const CHECK = (
  <svg viewBox="0 0 10 10" aria-hidden="true">
    <path d="m2 5.2 2 2L8 3" />
  </svg>
);

const CALENDAR = (
  <svg className="ribbon__glyph" viewBox="0 0 12 12" aria-hidden="true">
    <rect x="1.5" y="2.5" width="9" height="8" rx="1" />
    <path d="M1.5 5h9M4 1.5v2M8 1.5v2" />
  </svg>
);

export function Ribbon({ ribbon, onOpenAgent, onOpenPile, openAgentId, onTrackWidth }: Props) {
  const agents = useStore((s) => s.agents);
  const activity = useStore((s) => s.activity);
  const byId = new Map(agents.map((agent) => [agent.id, agent]));

  const stops = [
    ...(ribbon.pile.length ? [PILE_KEY] : []),
    ...[
      ...ribbon.faces.map((face) => ({ x: face.x, key: face.agentId as string })),
      ...ribbon.counts.map((count) => ({ x: count.x, key: count.key })),
    ]
      .sort((a, b) => a.x - b.x)
      .map((one) => one.key),
  ];
  const [current, setCurrent] = useState<string | null>(null);
  const active = current !== null && stops.includes(current) ? current : (stops[0] ?? null);
  const buttons = useRef(new Map<string, HTMLButtonElement>());

  const track = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const node = track.current;
    if (!node || !onTrackWidth) return;
    onTrackWidth(node.getBoundingClientRect().width);
    const watch = new ResizeObserver(([entry]) => {
      if (entry) onTrackWidth(entry.contentRect.width);
    });
    watch.observe(node);
    return () => watch.disconnect();
  }, [onTrackWidth]);

  const move = (event: KeyboardEvent) => {
    if (active === null) return;
    const from = stops.indexOf(active);
    const to =
      event.key === "ArrowRight"
        ? Math.min(stops.length - 1, from + 1)
        : event.key === "ArrowLeft"
          ? Math.max(0, from - 1)
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? stops.length - 1
              : null;
    if (to === null) return;
    event.preventDefault();
    const key = stops[to] as string;
    setCurrent(key);
    buttons.current.get(key)?.focus();
  };

  const keep = (key: string) => (node: HTMLButtonElement | null) => {
    if (node) buttons.current.set(key, node);
    else buttons.current.delete(key);
  };

  const drawn: { x: number; node: ReactNode }[] = [
    ...ribbon.lapses.map((lapse) => ({
      x: lapse.x,
      node: (
        <li
          key={`lapse:${lapse.approvalId}`}
          className="ribbon__lapse"
          style={at(lapse.x)}
          title={`${lapse.label}, ${lapse.when}`}
        >
          <span className="ribbon__said">{`${lapse.label}, ${lapse.when}`}</span>
        </li>
      ),
    })),
    ...ribbon.scheduled.map((item) => {
      const owner = item.agentId ? byId.get(item.agentId) : undefined;
      return {
        x: item.x,
        node: (
          <li
            key={item.key}
            className="ribbon__item"
            style={{ ...at(item.x), "--ribbon-room": item.room } as CSSProperties}
            data-words={item.words ? undefined : "none"}
            data-align={item.align}
            title={item.label}
          >
            <span className="ribbon__item-mark" aria-hidden="true">
              {owner ? (
                <AgentAvatar
                  avatar={owner.avatar}
                  color={owner.color}
                  lifecycle={owner.lifecycle}
                  seed={owner.id}
                  size="sm"
                  title=""
                />
              ) : (
                CALENDAR
              )}
            </span>
            {item.words && (
              <span className="ribbon__item-what" aria-hidden="true">
                {item.what}
              </span>
            )}
            <span className="ribbon__item-when" aria-hidden="true">
              {item.when}
            </span>
            <span className="ribbon__said">{item.label}</span>
          </li>
        ),
      };
    }),
    ...ribbon.faces.flatMap((face) => {
      const agent = byId.get(face.agentId);
      if (!agent) return [];
      return {
        x: face.x,
        node: (
          <li
            key={face.agentId}
            className="ribbon__face"
            style={at(face.x)}
            data-state={face.state}
            data-align={face.align}
            data-open={face.agentId === openAgentId ? "" : undefined}
          >
            <button
              type="button"
              ref={keep(face.agentId)}
              className="ribbon__face-button"
              aria-label={face.label}
              aria-current={face.agentId === openAgentId ? "page" : undefined}
              tabIndex={active === face.agentId ? 0 : -1}
              onFocus={() => setCurrent(face.agentId)}
              onClick={() => onOpenAgent(face.agentId)}
            >
              <span className="ribbon__portrait">
                <AgentAvatar
                  avatar={agent.avatar}
                  color={agent.color}
                  activity={activity[agent.id]}
                  lifecycle={agent.lifecycle}
                  seed={agent.id}
                  size="md"
                  title=""
                />
                {face.state === "needsYou" && <span className="ribbon__alert">!</span>}
                {(face.state === "finished" || face.state === "failed") && (
                  <span className="ribbon__mark" data-mark={face.state}>
                    {face.state === "failed" ? "!" : CHECK}
                  </span>
                )}
              </span>
              <span className="ribbon__name">{agent.name}</span>
            </button>
          </li>
        ),
      };
    }),
    ...ribbon.counts.map((count) => ({
      x: count.x,
      node: (
        <li
          key={count.key}
          className="ribbon__count"
          style={at(count.x)}
          data-kind={count.kind}
          data-align={count.align}
        >
          <button
            type="button"
            ref={keep(count.key)}
            className="ribbon__count-button"
            aria-label={count.label}
            tabIndex={active === count.key ? 0 : -1}
            onFocus={() => setCurrent(count.key)}
            onClick={onOpenPile}
          >
            {count.text}
          </button>
        </li>
      ),
    })),
  ].sort((a, b) => a.x - b.x);

  const pileShown = ribbon.pile.slice(0, PILE_SHOWN);
  const marked = ribbon.faces.some((face) => face.agentId === openAgentId);

  return (
    <nav
      className="ribbon"
      aria-label="Agents"
      data-pile={ribbon.pile.length ? "some" : "none"}
      data-open={marked ? "some" : undefined}
      style={{ "--ribbon-pile": pileShown.length } as CSSProperties}
    >
      <div className="ribbon__inner">
        <div className="ribbon__line" aria-hidden="true" />
        <div className="ribbon__track" ref={track} aria-hidden="true">
          {ribbon.band && (
            <div
              className="ribbon__band"
              style={{ "--ribbon-x": ribbon.band.x, "--ribbon-to": ribbon.now.x } as CSSProperties}
            >
              {ribbon.band.label && <span className="ribbon__band-label">{ribbon.band.label}</span>}
            </div>
          )}
          {ribbon.ticks.map((tick) => (
            <span
              key={`${tick.kind}:${tick.at}`}
              className={`ribbon__tick ribbon__tick--${tick.kind}`}
              style={at(tick.x)}
            >
              {tick.label}
            </span>
          ))}
          <div className="ribbon__now" style={at(ribbon.now.x)} />
          <span className="ribbon__pill" style={at(ribbon.now.x)}>
            {ribbon.now.label}
          </span>
        </div>

        <ul className="ribbon__list" aria-label="Today's timeline" onKeyDown={move}>
          {ribbon.pile.length > 0 && (
            <li className="ribbon__pile">
              <button
                type="button"
                ref={keep(PILE_KEY)}
                className="ribbon__pile-button"
                tabIndex={active === PILE_KEY ? 0 : -1}
                onFocus={() => setCurrent(PILE_KEY)}
                onClick={onOpenPile}
              >
                <span className="ribbon__pile-faces" aria-hidden="true">
                  {pileShown.map((id) => {
                    const agent = byId.get(id);
                    return agent ? (
                      <AgentAvatar
                        key={id}
                        avatar={agent.avatar}
                        color={agent.color}
                        lifecycle={agent.lifecycle}
                        seed={id}
                        size="sm"
                        title=""
                      />
                    ) : null;
                  })}
                </span>
                <span className="ribbon__pile-label">
                  {ribbon.pile.length} {ribbon.crowded ? "more" : "asleep"}
                </span>
              </button>
            </li>
          )}

          {drawn.map((one) => one.node)}
        </ul>
      </div>
    </nav>
  );
}
