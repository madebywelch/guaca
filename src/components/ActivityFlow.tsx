import { useMemo, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import type { AgentCard, AgentId, Envelope, Participant } from "../lib/types";
import { plainText } from "../lib/types";
import { MessageModal, type WirePeer } from "./WireRow";

/**
 * The conversation as a sequence diagram.
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
 * and the rest of the row says what. The board is readable without touching it,
 * and a relay chain reads as a staircase.
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
}

interface Row {
  message: Envelope;
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

  const { lanes, rows } = useMemo(() => {
    const lanes: Lane[] = [];
    const index = new Map<string, number>();

    // Lanes appear in the order participants first speak or are spoken to, so
    // the board's shape reflects how the conversation actually spread.
    const lane = (participant: Participant): number => {
      const key = laneKey(participant);
      const existing = index.get(key);
      if (existing !== undefined) return existing;

      const peer =
        participant.kind === "agent"
          ? peerFor(participant.id, byId)
          : participant.kind === "human"
            ? YOU
            : { id: "system", name: "Guac", color: "#8a5a2f", avatar: "plain" };

      index.set(key, lanes.length);
      lanes.push({ key, peer });
      return lanes.length - 1;
    };

    let run: string | null = null;
    const rows: Row[] = messages.map((message) => {
      const startsRun = message.runId !== run;
      run = message.runId;
      return {
        message,
        fromLane: lane(message.from),
        toLane: lane(message.to),
        startsRun,
      };
    });

    return { lanes, rows };
  }, [messages, byId]);

  if (rows.length === 0) {
    return (
      <div className="empty">
        <p className="empty__body">
          Nothing has happened yet. Send an agent a message and the conversation will draw itself
          here: who spoke to whom, in the order it happened.
        </p>
      </div>
    );
  }

  const boardWidth = Math.max(lanes.length * LANE_W, LANE_W);
  const laneX = (i: number) => i * LANE_W + LANE_W / 2;
  const open = openIndex === null ? null : rows[openIndex];

  return (
    <div className="flow">
      <div className="flow__head">
        <div className="flow__lanes" style={{ width: boardWidth }}>
          {lanes.map((lane) => (
            <div className="flow__lane" key={lane.key} style={{ width: LANE_W }}>
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
        <span className="flow__head-note">what was said</span>
      </div>

      <div className="flow__scroll">
        <div className="flow__body">
          {/* One continuous rail per participant, behind the rows, so a column
              reads as a lifeline rather than as a stack of unrelated arrows. */}
          <div className="flow__rails" style={{ width: boardWidth }} aria-hidden="true">
            {lanes.map((lane, i) => (
              <span key={lane.key} className="flow__rail" style={{ left: laneX(i) }} />
            ))}
          </div>

          {rows.map((row, i) => {
            const { message } = row;
            const sender = message.from.kind === "agent" ? peerFor(message.from.id, byId) : YOU;
            const target = message.to.kind === "agent" ? peerFor(message.to.id, byId) : YOU;
            const body = plainText(message);
            const rightwards = row.toLane >= row.fromLane;
            const x1 = laneX(row.fromLane);
            const x2 = laneX(row.toLane);
            const label = `${sender.name} to ${target.name}`;

            return (
              <div key={message.id}>
                {row.startsRun && (
                  <div className="flow__divider">
                    <span>{clockTime(message.createdAt)}</span>
                  </div>
                )}

                <button
                  type="button"
                  className="flow__row"
                  aria-label={label}
                  onClick={() => setOpenIndex(i)}
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
                    <line x1={x1} y1={13} x2={x2} y2={13} stroke={sender.color} strokeWidth="1.6" />
                    <circle cx={x1} cy={13} r="3" fill={sender.color} />
                    {/* Drawn rather than a marker so it can point either way
                        without a second marker definition per colour. */}
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
              </div>
            );
          })}
        </div>
      </div>

      {open && (
        <MessageModal
          title={`${
            open.message.from.kind === "agent" ? peerFor(open.message.from.id, byId).name : "You"
          } → ${open.message.to.kind === "agent" ? peerFor(open.message.to.id, byId).name : "You"}`}
          peer={open.message.from.kind === "agent" ? peerFor(open.message.from.id, byId) : YOU}
          counterpart={open.message.to.kind === "agent" ? peerFor(open.message.to.id, byId) : YOU}
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

/**
 * A participant to draw. Deleted agents still get a lane: their messages are
 * part of what happened, and dropping them would leave arrows pointing nowhere.
 */
function peerFor(id: AgentId, byId: (id: AgentId) => AgentCard | undefined): WirePeer {
  const card = byId(id);
  return card
    ? { id: card.id, name: card.name, color: card.color, avatar: card.avatar }
    : { id, name: "Deleted agent", color: "#8aa0a6", avatar: "pit" };
}
