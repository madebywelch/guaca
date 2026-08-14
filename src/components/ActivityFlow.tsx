import { useEffect, useMemo, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import type { AgentCard, AgentId, Envelope, Participant } from "../lib/types";
import { plainText } from "../lib/types";
import { MessageModal, type WirePeer } from "./WireRow";

/**
 * The conversation as a flow board.
 *
 * A vertical list cannot answer the question this view exists for: who spoke to
 * whom, in what order, and what set it off. Reading causality out of a column
 * of chips means holding names and hop numbers in your head.
 *
 * So this is a sequence diagram turned on its side. Each participant owns a
 * lane; time runs left to right; every message is an arrow from one lane to
 * another at the moment it was sent. Following a conversation becomes reading
 * left to right, and a relay chain becomes a visible staircase.
 */

const LANE_H = 54;
const COL_W = 92;
const TOP_PAD = 40;
const BOTTOM_PAD = 18;

const YOU: WirePeer = { id: "human", name: "You", color: "#5b665e", avatar: "plain" };

interface Props {
  messages: Envelope[];
  byId: (id: AgentId) => AgentCard | undefined;
}

interface Lane {
  key: string;
  peer: WirePeer;
}

interface Node {
  message: Envelope;
  column: number;
  fromLane: number;
  toLane: number;
  /** True at the first message of a run: a new thing the operator started. */
  startsRun: boolean;
}

function laneKey(participant: Participant): string {
  return participant.kind === "agent" ? participant.id : participant.kind;
}

function clockTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function ActivityFlow({ messages, byId }: Props) {
  const [openIndex, setOpenIndex] = useState<number | null>(null);
  const [hovered, setHovered] = useState<number | null>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const pinnedRight = useRef(true);

  const { lanes, nodes } = useMemo(() => {
    // Lanes appear in the order they first speak or are spoken to, which reads
    // as the order the conversation brought them in.
    const order: Lane[] = [];
    const index = new Map<string, number>();

    const laneFor = (participant: Participant): number => {
      const key = laneKey(participant);
      const existing = index.get(key);
      if (existing !== undefined) return existing;

      const peer =
        participant.kind === "human"
          ? YOU
          : participant.kind === "system"
            ? { id: "system", name: "Guac", color: "#8a5a2f", avatar: "plain" }
            : peerFor(participant.id, byId);

      index.set(key, order.length);
      order.push({ key, peer });
      return order.length - 1;
    };

    let previousRun: string | null = null;
    const built: Node[] = messages.map((message, column) => {
      const startsRun = message.runId !== previousRun;
      previousRun = message.runId;
      return {
        message,
        column,
        fromLane: laneFor(message.from),
        toLane: laneFor(message.to),
        startsRun,
      };
    });

    return { lanes: order, nodes: built };
  }, [messages, byId]);

  const width = Math.max(nodes.length * COL_W + COL_W, COL_W * 3);
  const height = TOP_PAD + lanes.length * LANE_H + BOTTOM_PAD;
  const laneY = (lane: number) => TOP_PAD + lane * LANE_H + LANE_H / 2;

  // New messages arrive at the right edge, so follow them unless the operator
  // has scrolled back to look at something.
  useEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    const onScroll = () => {
      pinnedRight.current = node.scrollWidth - node.scrollLeft - node.clientWidth < 120;
    };
    node.addEventListener("scroll", onScroll, { passive: true });
    return () => node.removeEventListener("scroll", onScroll);
  }, []);

  useEffect(() => {
    const node = scrollRef.current;
    if (node && pinnedRight.current) node.scrollLeft = node.scrollWidth;
  }, [nodes.length]);

  if (nodes.length === 0) {
    return (
      <div className="empty">
        <p className="empty__body">
          Nothing has happened yet. Send an agent a message and the conversation will draw itself
          here: who spoke to whom, in the order it happened.
        </p>
      </div>
    );
  }

  const open = openIndex === null ? null : nodes[openIndex];

  return (
    <div className="flow">
      <div className="flow__lanes" style={{ paddingTop: TOP_PAD }}>
        {lanes.map((lane) => (
          <div className="flow__lane" key={lane.key} style={{ height: LANE_H }}>
            <AgentAvatar
              avatar={lane.peer.avatar}
              color={lane.peer.color}
              size="sm"
              seed={lane.peer.id}
            />
            <span className="flow__name">{lane.peer.name}</span>
          </div>
        ))}
      </div>

      <div className="flow__scroll" ref={scrollRef}>
        <svg width={width} height={height} className="flow__board">
          <title>Conversation flow</title>

          {lanes.map((lane, i) => (
            <line
              key={lane.key}
              x1={0}
              x2={width}
              y1={laneY(i)}
              y2={laneY(i)}
              className="flow__rail"
            />
          ))}

          {nodes.map((node, i) => {
            const x = COL_W / 2 + node.column * COL_W;
            const y1 = laneY(node.fromLane);
            const y2 = laneY(node.toLane);
            const down = y2 > y1;
            const tip = down ? y2 - 7 : y2 + 7;
            const color =
              node.message.from.kind === "agent"
                ? peerFor(node.message.from.id, byId).color
                : YOU.color;
            const active = hovered === i || openIndex === i;
            const text = plainText(node.message);

            return (
              // SVG has no interactive primitive: a <button> would have to live
              // in a foreignObject that cannot be positioned against the
              // drawing. Role plus tabindex is the standard way to make a shape
              // operable.
              // biome-ignore lint/a11y/useSemanticElements: SVG has no <button>
              <g
                key={node.message.id}
                className="flow__node"
                data-active={active}
                onMouseEnter={() => setHovered(i)}
                onMouseLeave={() => setHovered(null)}
                onClick={() => setOpenIndex(i)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" || event.key === " ") setOpenIndex(i);
                }}
                role="button"
                tabIndex={0}
                aria-label={`${laneName(lanes, node.fromLane)} to ${laneName(
                  lanes,
                  node.toLane,
                )} at ${clockTime(node.message.createdAt)}`}
              >
                {node.startsRun && (
                  <line
                    x1={x - COL_W / 2}
                    x2={x - COL_W / 2}
                    y1={6}
                    y2={height - 6}
                    className="flow__divider"
                  />
                )}

                <line x1={x} x2={x} y1={y1} y2={tip} stroke={color} className="flow__wire" />
                <circle cx={x} cy={y1} r={4.5} fill={color} />
                <path
                  d={down ? `M${x} ${y2 - 1}l-5-7h10z` : `M${x} ${y2 + 1}l-5 7h10z`}
                  fill={color}
                />

                {active && (
                  <text x={x + 9} y={(y1 + y2) / 2} className="flow__excerpt">
                    {text.slice(0, 28)}
                    {text.length > 28 ? "…" : ""}
                  </text>
                )}

                <text x={x} y={TOP_PAD - 22} className="flow__time" textAnchor="middle">
                  {clockTime(node.message.createdAt)}
                </text>
                <text x={x} y={TOP_PAD - 10} className="flow__hop" textAnchor="middle">
                  {node.message.hop > 0 ? `hop ${node.message.hop}` : "you"}
                </text>

                {/* A generous hit area: the arrow itself is a few pixels wide. */}
                <rect
                  x={x - COL_W / 2}
                  y={0}
                  width={COL_W}
                  height={height}
                  fill="transparent"
                  className="flow__hit"
                />
              </g>
            );
          })}
        </svg>
      </div>

      {open && (
        <MessageModal
          title={`${laneName(lanes, open.fromLane)} → ${laneName(lanes, open.toLane)}`}
          peer={lanes[open.fromLane]!.peer}
          counterpart={lanes[open.toLane]!.peer}
          hop={open.message.hop}
          at={open.message.createdAt}
          body={plainText(open.message)}
          refusal={null}
          onClose={() => setOpenIndex(null)}
        />
      )}
    </div>
  );
}

function laneName(lanes: Lane[], index: number): string {
  return lanes[index]?.peer.name ?? "?";
}

function peerFor(id: AgentId, byId: Props["byId"]): WirePeer {
  const card = byId(id);
  return card
    ? { id: card.id, name: card.name, color: card.color, avatar: card.avatar }
    : { id, name: "Deleted agent", color: "#8aa0a6", avatar: "blank" };
}
