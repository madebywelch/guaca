/**
 * A channel, arranged the way it is read.
 *
 * Three kinds of thing land in one agent's channel and they want three
 * different amounts of room:
 *
 * - what the operator and that agent said to each other: full bubbles, the
 *   only messages written to be read in full
 * - what the agent exchanged with its peers: one centred line per peer, with
 *   the exchange itself a click away
 * - what the agent did on its own turn: tool trails, guard stops, refusals
 *
 * The middle one is why this is a pass over the list rather than a decision
 * per message. Peer traffic arrives in bursts (a fan-out, then the answers
 * landing milliseconds apart) and a line each buries the operator's own
 * conversation just as thoroughly as bubbles did, only in smaller type. So a
 * burst collapses to one row saying how much of it there was and with whom,
 * and anything that is not peer traffic ends the burst, because a tool trail
 * between two exchanges is a break in what was happening.
 *
 * Counts are of what this channel holds, which is what the operator is
 * looking at. An agent's automatic replies are filed in the channel of
 * whoever they went to, so the thread behind the row can hold more than the
 * row counted. That is the right way round: clicking reveals more, never
 * less.
 */

import { sendBody, sendRecipients } from "./toolArgs";
import type { AgentCard, AgentId, Envelope, MessageId, Part, ToolOutcome } from "./types";

/**
 * Resolves agents for rendering history. Deleted agents resolve too: they are
 * still in transcripts, and a message from a nameless id is unreadable.
 */
export interface Lookups {
  byId: (id: AgentId) => AgentCard | undefined;
  /** Tool calls name their recipients, so names need resolving too. */
  byName: (name: string) => AgentCard | undefined;
}

/** An agent as a row draws it. */
export interface WirePeer {
  name: string;
  color: string;
  avatar: string;
  id: string;
}

/**
 * Resolves an agent to the shape a row needs.
 *
 * `fallbackName` matters: a tool call names its recipients, and a name that
 * never resolved is far more useful shown as written than as "Deleted agent".
 * An agent that wrote to "Nobody" should read as having written to Nobody.
 */
export function toPeer(
  card: AgentCard | undefined,
  fallbackId: string,
  fallbackName = "Deleted agent",
): WirePeer {
  return card
    ? { id: card.id, name: card.name, color: card.color, avatar: card.avatar }
    : { id: fallbackId, name: fallbackName, color: "#8aa0a6", avatar: "blank" };
}

/** One peer's share of a burst of traffic. */
export interface PeerSummary {
  peer: WirePeer;
  /**
   * Whose thread the row opens. Null when a recipient's name never resolved to
   * an agent: there is a name worth showing and no conversation to read.
   */
  agentId: AgentId | null;
  sent: number;
  received: number;
}

/** One entry in a channel, in the order it is drawn. */
export type Row =
  /** An envelope rendered whole: a bubble, a request, a tool trail. */
  | { kind: "message"; key: string; message: Envelope; continued: boolean }
  /**
   * A run of peer traffic, one summary per peer involved. `folded` names the
   * messages it stands for, so something looking for one of them can be sent
   * to the row that swallowed it.
   */
  | { kind: "peers"; key: string; peers: PeerSummary[]; folded: MessageId[] }
  /** A send the runtime stopped. Never folded into a burst: it did not happen. */
  | {
      kind: "refused";
      key: string;
      peer: WirePeer;
      at: number;
      body: string;
      reason: string;
    }
  /**
   * When the conversation picked up again, after a gap long enough that the
   * messages either side of it are not the same sitting.
   */
  | { kind: "when"; key: string; at: number };

/** How long a gap can be before a second message from the same author gets its own header. */
const CONTINUATION_MS = 4 * 60 * 1000;

/**
 * How long a silence has to be before the transcript says when it ended.
 *
 * This is what a per-message clock was for, and a clock on every message is a
 * column of near-identical numbers down a transcript where four in a row were
 * written in the same minute. The gaps are the part worth reading: the exact
 * time of any one message is still a hover away.
 */
const QUIET_MS = 30 * 60 * 1000;

/** True when consecutive messages should merge under one header. */
export function continues(previous: Envelope | undefined, current: Envelope): boolean {
  if (!previous) return false;
  if (JSON.stringify(previous.from) !== JSON.stringify(current.from)) return false;
  if (JSON.stringify(previous.to) !== JSON.stringify(current.to)) return false;
  return current.createdAt - previous.createdAt < CONTINUATION_MS;
}

/**
 * Why one recipient of a send did not get it, per recipient.
 *
 * A fan-out can be half-delivered. Painting one verdict across every name drew
 * agents that were refused as "sent to", which is the one thing this trail
 * exists to be right about.
 */
function refusalFor(outcome: ToolOutcome, name: string): string | null {
  if (outcome.status === "refused") return outcome.reason;
  if (outcome.status === "failed") return outcome.error;
  if (outcome.status === "partial") {
    return outcome.refused.find((r) => r.to === name)?.reason ?? null;
  }
  return null;
}

export function transcriptRows(messages: Envelope[], lookups: Lookups): Row[] {
  const rows: Row[] = [];
  /** The burst still open, which is to say the last row pushed. */
  let burst: Extract<Row, { kind: "peers" }> | null = null;
  /** The last bubble, for merging a follow-up under one header. */
  let spoken: Envelope | undefined;
  /** When the last message was, for spotting the silence after it. */
  let lastAt: number | null = null;
  /**
   * A gap that has been earned but not yet drawn.
   *
   * Held rather than pushed on sight, because a message does not always produce
   * a row: a turn whose every part was folded into a burst leaves none. Emitted
   * immediately before whatever row does come, so the line always has something
   * underneath it and never trails the transcript.
   */
  let gap: { at: number; key: string } | null = null;

  const mark = () => {
    if (!gap) return;
    rows.push({ kind: "when", key: `when:${gap.key}`, at: gap.at });
    gap = null;
  };

  const count = (
    peer: WirePeer,
    agentId: AgentId | null,
    direction: "sent" | "received",
    key: string,
    /**
     * The message this row now stands for on screen, where there is one. A
     * send made through a tool call has none: it is filed in the recipient's
     * channel, and what this one holds is the sender's record of making it.
     */
    folded?: MessageId,
  ) => {
    let open = burst;
    if (!open) {
      mark();
      open = { kind: "peers", key: `peers:${key}`, peers: [], folded: [] };
      rows.push(open);
      burst = open;
    }
    if (folded) open.folded.push(folded);
    let held = open.peers.find((entry) => entry.peer.id === peer.id);
    if (!held) {
      held = { peer, agentId, sent: 0, received: 0 };
      open.peers.push(held);
    }
    if (direction === "sent") held.sent += 1;
    else held.received += 1;
  };

  /** Anything that is not peer traffic. Ends the burst it interrupts. */
  const interrupt = (row: Row) => {
    mark();
    rows.push(row);
    burst = null;
  };

  for (const message of messages) {
    const { from, to } = message;

    // A silence long enough to be a break also ends whatever burst was open:
    // two exchanges three hours apart are two things that happened, not one.
    if (lastAt !== null && message.createdAt - lastAt >= QUIET_MS) {
      gap = { at: message.createdAt, key: message.id };
      burst = null;
    }
    lastAt = message.createdAt;

    // Ahead of everything else: a permission request is addressed to the
    // operator whoever the envelope says it is from, and it is the one thing
    // in a transcript they are expected to act on rather than read.
    if (message.parts.some((part) => part.type === "approval")) {
      interrupt({ kind: "message", key: message.id, message, continued: false });
      spoken = undefined;
      continue;
    }

    if (from.kind === "agent" && to.kind === "agent") {
      count(toPeer(lookups.byId(from.id), from.id), from.id, "received", message.id, message.id);
      spoken = undefined;
      continue;
    }

    // The agent's own record of its turn. Sends are peer traffic wearing a
    // tool call; everything else is the trail it left.
    if (from.kind === "agent" && to.kind === "system") {
      const trail: Part[] = [];
      const refused: Row[] = [];

      for (const part of message.parts) {
        const names =
          part.type === "toolCall" && part.name === "send_message"
            ? sendRecipients(part.arguments)
            : [];
        // A send nobody can be named on is not traffic between two agents. It
        // stays in the trail, where the reason it went nowhere is legible.
        if (part.type !== "toolCall" || names.length === 0) {
          trail.push(part);
          continue;
        }

        const body = sendBody(part.arguments);
        for (const name of names) {
          const card = lookups.byName(name);
          const reason = refusalFor(part.outcome, name);
          if (reason) {
            refused.push({
              kind: "refused",
              key: `${message.id}:${name}`,
              peer: toPeer(card, name, name),
              at: message.createdAt,
              body,
              reason,
            });
          } else {
            count(toPeer(card, name, name), card?.id ?? null, "sent", `${message.id}:${name}`);
          }
        }
      }

      for (const row of refused) interrupt(row);
      if (trail.length > 0) {
        interrupt({
          kind: "message",
          key: `${message.id}:trail`,
          message: { ...message, parts: trail },
          continued: false,
        });
      }
      spoken = undefined;
      continue;
    }

    interrupt({ kind: "message", key: message.id, message, continued: continues(spoken, message) });
    spoken = message;
  }

  return rows;
}

/**
 * Which messages a row is the on-screen home of.
 *
 * More than one for a burst, which is the point of it: search finds a message
 * by its text and hands back an id, and an agent-to-agent message has no row
 * of its own here any more. Landing on the row that collapsed it is the honest
 * answer, since the row is where that message is in this channel and opening
 * the thread is how it is read.
 */
export function rowStandsFor(row: Row): MessageId[] {
  if (row.kind === "peers") return row.folded;
  if (row.kind === "refused" || row.kind === "when") return [];
  return [row.message.id];
}

/** How a burst reads for one peer: the count, or what the single message was. */
export function summaryLabel(summary: PeerSummary): string {
  const total = summary.sent + summary.received;
  if (total === 1)
    return summary.sent === 1
      ? `Messaged ${summary.peer.name}`
      : `Message from ${summary.peer.name}`;
  return `${total} messages with ${summary.peer.name}`;
}
