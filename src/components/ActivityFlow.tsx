import { useEffect, useMemo, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { api } from "../lib/ipc";
import type { AgentCard, AgentId, Envelope, Participant, RunId, Tokens } from "../lib/types";
import { plainText } from "../lib/types";
import { compact, money } from "./TokenMeter";
import { MessageModal, type WirePeer } from "./WireRow";

/**
 * The conversation as a sequence diagram, one board per run.
 *
 * The question this view exists to answer is who spoke to whom, in what order,
 * and what set it off. An earlier pass drew it sideways — lanes stacked, time
 * running left to right — which is a real sequence diagram but leaves each
 * message a bare arrow in a 92px column. There is no room for a single word, so
 * every message had to be clicked to be read, and following a conversation
 * meant clicking along it one arrow at a time.
 *
 * Turning it upright is what makes it legible. Participants are columns, time
 * runs down, and each message owns a whole row: the arrow says who and to whom,
 * and the rest of the row says what. A relay chain reads as a staircase.
 *
 * Two things then went wrong as a workspace aged, both fixed by cutting the
 * board into runs. Lanes were global, so every agent that ever spoke held a
 * column forever: adding agents pushed the newcomers rightward and stretched
 * every arrow across the dead columns of agents that had been deleted months
 * ago. And the whole history was one scroll, so the thing an operator almost
 * always wants — what just happened — was at the very bottom.
 *
 * So: a run is a board, a board draws only the participants in it, and the
 * newest is first and open. History is still all here, one line each.
 */

/** Width of one participant column. */
const LANE_W = 72;

const YOU: WirePeer = { id: "human", name: "You", color: "#5b665e", avatar: "plain" };

interface Props {
  messages: Envelope[];
  byId: (id: AgentId) => AgentCard | undefined;
}

interface Lane {
  key: string;
  peer: WirePeer;
  /** A deleted agent. Its traffic is history, but it is not someone you have. */
  gone: boolean;
}

interface Row {
  message: Envelope;
  fromLane: number;
  toLane: number;
}

interface Block {
  key: string;
  runId: RunId;
  startedAt: number;
  rows: Row[];
  lanes: Lane[];
  /** What set the run off, for reading the history without opening it. */
  opener: string;
}

function laneKey(participant: Participant): string {
  return participant.kind === "agent" ? participant.id : participant.kind;
}

function clockTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** Only says the day when it is not today, since most reading is of today. */
function dayLabel(ms: number): string {
  const at = new Date(ms);
  const today = new Date();
  const sameDay =
    at.getFullYear() === today.getFullYear() &&
    at.getMonth() === today.getMonth() &&
    at.getDate() === today.getDate();
  return sameDay ? "" : at.toLocaleDateString([], { month: "short", day: "numeric" });
}

export function ActivityFlow({ messages, byId }: Props) {
  const [open, setOpen] = useState<{ block: number; row: number } | null>(null);
  // Absent means "the default", which is open for the newest board and closed
  // for the rest. Storing the exception rather than the state means a new run
  // arriving does not have to be reconciled with anything.
  const [toggled, setToggled] = useState<Record<string, boolean>>({});

  const blocks = useMemo(() => buildBlocks(messages, byId), [messages, byId]);

  // What each run cost. Asked for once per set of runs on screen rather than
  // carried on every message, which would repeat one run's totals as many
  // times as the run had messages.
  const [spent, setSpent] = useState<Record<RunId, Tokens | undefined>>({});
  const runs = useMemo(() => blocks.map((b) => b.runId).join(","), [blocks]);
  useEffect(() => {
    if (!runs) return;
    let live = true;
    void api.usageForRuns(runs.split(",") as RunId[]).then((rows) => {
      if (!live) return;
      const next: Record<RunId, Tokens> = {};
      for (const row of rows) {
        next[row.runId] = {
          prompt: row.prompt,
          completion: row.completion,
          cost: row.cost,
          calls: row.calls,
        };
      }
      setSpent(next);
    });
    return () => {
      live = false;
    };
  }, [runs]);

  if (blocks.length === 0) {
    return (
      <div className="empty">
        <p className="empty__body">
          Nothing has happened yet. Send an agent a message and the conversation will draw itself
          here: who spoke to whom, in the order it happened.
        </p>
      </div>
    );
  }

  const opened = open ? blocks[open.block]?.rows[open.row] : null;
  const openedMessage = opened?.message;

  return (
    <div className="flow">
      <div className="flow__scroll">
        {blocks.map((block, blockIndex) => {
          const isOpen = toggled[block.key] ?? blockIndex === 0;
          const boardWidth = Math.max(block.lanes.length * LANE_W, LANE_W);
          const laneX = (i: number) => i * LANE_W + LANE_W / 2;
          const day = dayLabel(block.startedAt);

          return (
            <section className="run" key={block.key} data-open={isOpen ? "true" : undefined}>
              <button
                type="button"
                className="run__head"
                onClick={() => setToggled((t) => ({ ...t, [block.key]: !isOpen }))}
                aria-expanded={isOpen}
              >
                <span className="run__caret" aria-hidden="true">
                  {isOpen ? "▾" : "▸"}
                </span>
                <span className="run__when">
                  {day && <span className="run__day">{day}</span>}
                  {clockTime(block.startedAt)}
                </span>
                <span className="run__faces" aria-hidden="true">
                  {block.lanes.map((lane) => (
                    <span className="run__face" key={lane.key} data-gone={lane.gone || undefined}>
                      <AgentAvatar
                        avatar={lane.peer.avatar}
                        color={lane.peer.color}
                        size="sm"
                        seed={lane.peer.id}
                      />
                    </span>
                  ))}
                </span>
                <span className="run__opener">{block.opener || "no text"}</span>
                <span className="run__count">
                  {block.rows.length} {block.rows.length === 1 ? "message" : "messages"}
                </span>
                {spent[block.runId] && (
                  <span
                    className="run__tokens"
                    title={`${spent[block.runId]!.prompt.toLocaleString()} in, ${spent[
                      block.runId
                    ]!.completion.toLocaleString()} out, over ${spent[
                      block.runId
                    ]!.calls.toLocaleString()} model call(s)`}
                  >
                    {compact(spent[block.runId]!.prompt + spent[block.runId]!.completion)}
                    {spent[block.runId]!.cost != null && (
                      <span className="run__cost">{money(spent[block.runId]!.cost!)}</span>
                    )}
                  </span>
                )}
              </button>

              {isOpen && (
                <div className="run__board">
                  <div className="flow__head">
                    <div className="flow__lanes" style={{ width: boardWidth }}>
                      {block.lanes.map((lane) => (
                        <div
                          className="flow__lane"
                          key={lane.key}
                          style={{ width: LANE_W }}
                          data-gone={lane.gone || undefined}
                        >
                          <AgentAvatar
                            avatar={lane.peer.avatar}
                            color={lane.peer.color}
                            size="sm"
                            seed={lane.peer.id}
                          />
                          <span className="flow__name">{lane.peer.name}</span>
                          {lane.gone && <span className="flow__gone">deleted</span>}
                        </div>
                      ))}
                    </div>
                    <span className="flow__head-note">what was said</span>
                  </div>

                  <div className="flow__body">
                    {/* One continuous rail per participant, behind the rows, so
                        a column reads as a lifeline rather than as a stack of
                        unrelated arrows. */}
                    <div className="flow__rails" style={{ width: boardWidth }} aria-hidden="true">
                      {block.lanes.map((lane, i) => (
                        <span key={lane.key} className="flow__rail" style={{ left: laneX(i) }} />
                      ))}
                    </div>

                    {block.rows.map((row, rowIndex) => {
                      const { message } = row;
                      const sender =
                        message.from.kind === "agent" ? peerFor(message.from.id, byId).peer : YOU;
                      const target =
                        message.to.kind === "agent" ? peerFor(message.to.id, byId).peer : YOU;
                      const body = plainText(message);
                      const rightwards = row.toLane >= row.fromLane;
                      const x1 = laneX(row.fromLane);
                      const x2 = laneX(row.toLane);
                      const label = `${sender.name} to ${target.name}`;

                      return (
                        <button
                          type="button"
                          key={message.id}
                          className="flow__row"
                          aria-label={label}
                          onClick={() => setOpen({ block: blockIndex, row: rowIndex })}
                          style={{ "--accent": sender.color } as React.CSSProperties}
                        >
                          <span className="flow__time">{clockTime(message.createdAt)}</span>

                          <svg
                            className="flow__arrow"
                            width={boardWidth}
                            height={26}
                            aria-hidden="true"
                            style={{ flex: "none" }}
                          >
                            <line
                              x1={x1}
                              y1={13}
                              x2={x2}
                              y2={13}
                              stroke={sender.color}
                              strokeWidth="1.6"
                            />
                            <circle cx={x1} cy={13} r="3" fill={sender.color} />
                            {/* Drawn rather than a marker so it can point either
                                way without a second marker per colour. */}
                            <path
                              d={rightwards ? `M${x2} 13l-6-4v8z` : `M${x2} 13l6-4v8z`}
                              fill={sender.color}
                            />
                          </svg>

                          <span className="flow__excerpt">
                            <span className="flow__who">{label}</span>
                            {body ? (
                              <span className="flow__said">{body}</span>
                            ) : (
                              <span className="flow__said flow__said--empty">no text</span>
                            )}
                          </span>

                          {message.hop > 0 && <span className="flow__hop">hop {message.hop}</span>}
                        </button>
                      );
                    })}
                  </div>
                </div>
              )}
            </section>
          );
        })}
      </div>

      {openedMessage && (
        <MessageModal
          title={`${
            openedMessage.from.kind === "agent"
              ? peerFor(openedMessage.from.id, byId).peer.name
              : "You"
          } → ${
            openedMessage.to.kind === "agent" ? peerFor(openedMessage.to.id, byId).peer.name : "You"
          }`}
          peer={
            openedMessage.from.kind === "agent" ? peerFor(openedMessage.from.id, byId).peer : YOU
          }
          counterpart={
            openedMessage.to.kind === "agent" ? peerFor(openedMessage.to.id, byId).peer : YOU
          }
          hop={openedMessage.hop}
          at={openedMessage.createdAt}
          body={plainText(openedMessage)}
          refusal={null}
          onClose={() => setOpen(null)}
        />
      )}
    </div>
  );
}

/**
 * Cuts the history into runs, newest first, each with its own lanes.
 *
 * Lanes are per run on purpose. Held globally they only ever grew, so a
 * workspace that had been used for a while drew every message across the dead
 * columns of agents nobody had spoken to in weeks.
 */
function buildBlocks(messages: Envelope[], byId: (id: AgentId) => AgentCard | undefined): Block[] {
  const blocks: Block[] = [];
  let current: Block | null = null;
  let index = new Map<string, number>();
  let run: string | null = null;

  for (const message of messages) {
    if (!current || message.runId !== run) {
      current = {
        key: message.id,
        runId: message.runId,
        startedAt: message.createdAt,
        rows: [],
        lanes: [],
        opener: plainText(message).slice(0, 140),
      };
      index = new Map();
      blocks.push(current);
      run = message.runId;
    }

    const block = current;
    const lane = (participant: Participant): number => {
      const key = laneKey(participant);
      const existing = index.get(key);
      if (existing !== undefined) return existing;

      const resolved =
        participant.kind === "agent"
          ? peerFor(participant.id, byId)
          : participant.kind === "human"
            ? { peer: YOU, gone: false }
            : {
                peer: { id: "system", name: "Guaca", color: "#8a5a2f", avatar: "plain" },
                gone: false,
              };

      index.set(key, block.lanes.length);
      block.lanes.push({ key, peer: resolved.peer, gone: resolved.gone });
      return block.lanes.length - 1;
    };

    block.rows.push({
      message,
      fromLane: lane(message.from),
      toLane: lane(message.to),
    });
  }

  // Newest first. What an operator wants from this view is almost always the
  // most recent thing, and it used to be the furthest scroll away.
  return blocks.reverse();
}

/**
 * A participant to draw. Deleted agents still get a lane: their messages are
 * part of what happened, and dropping them would leave arrows pointing nowhere.
 * They are marked, because two agents that shared a name are otherwise
 * indistinguishable, which is exactly when this view is being read.
 */
function peerFor(
  id: AgentId,
  byId: (id: AgentId) => AgentCard | undefined,
): { peer: WirePeer; gone: boolean } {
  const card = byId(id);
  if (!card) {
    return { peer: { id, name: "Deleted agent", color: "#8aa0a6", avatar: "pit" }, gone: true };
  }
  return {
    peer: { id: card.id, name: card.name, color: card.color, avatar: card.avatar },
    gone: card.lifecycle === "terminated",
  };
}
