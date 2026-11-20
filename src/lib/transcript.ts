import { ABSENT_ACCENT } from "../avatars/catalog";
import { sendBody, sendRecipients } from "./toolArgs";
import type { AgentCard, AgentId, Envelope, MessageId, Part, RunId, ToolOutcome } from "./types";
import { plainText } from "./types";

export interface Lookups {
  byId: (id: AgentId) => AgentCard | undefined;
  byName: (name: string) => AgentCard | undefined;
}

export interface WirePeer {
  name: string;
  color: string;
  avatar: string;
  id: string;
}

export function toPeer(
  card: AgentCard | undefined,
  fallbackId: string,
  fallbackName = "Deleted agent",
): WirePeer {
  return card
    ? { id: card.id, name: card.name, color: card.color, avatar: card.avatar }
    : { id: fallbackId, name: fallbackName, color: ABSENT_ACCENT, avatar: "blank" };
}

export interface PeerSummary {
  peer: WirePeer;
  agentId: AgentId | null;
  sent: number;
  received: number;
  gist: string | null;
  asked: string | null;
  askedAt: number | null;
  answer: string | null;
  answeredAt: number | null;
}

export type Row =
  | { kind: "message"; key: string; message: Envelope; continued: boolean }
  | {
      kind: "peers";
      key: string;
      runId: RunId;
      peers: PeerSummary[];
      folded: MessageId[];
      at: number;
    }
  | {
      kind: "refused";
      key: string;
      peer: WirePeer;
      at: number;
      body: string;
      reason: string;
    }
  | { kind: "when"; key: string; at: number }
  | {
      kind: "result";
      key: string;
      message: Envelope;
      handoff: Extract<Row, { kind: "peers" }> | null;
    };

const CONTINUATION_MS = 4 * 60 * 1000;

const QUIET_MS = 30 * 60 * 1000;

export function continues(previous: Envelope | undefined, current: Envelope): boolean {
  if (!previous) return false;
  if (JSON.stringify(previous.from) !== JSON.stringify(current.from)) return false;
  if (JSON.stringify(previous.to) !== JSON.stringify(current.to)) return false;
  return current.createdAt - previous.createdAt < CONTINUATION_MS;
}

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
  let burst: Extract<Row, { kind: "peers" }> | null = null;
  let spoken: Envelope | undefined;
  let lastAt: number | null = null;
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
    at: number,
    runId: RunId,
    folded: MessageId | undefined,
    said: string | null,
  ) => {
    const question =
      direction === "received"
        ? [...rows]
            .reverse()
            .find(
              (row): row is Extract<Row, { kind: "peers" }> =>
                row.kind === "peers" &&
                row.runId === runId &&
                row.peers.some(
                  (entry) =>
                    entry.peer.id === peer.id && entry.asked !== null && entry.answer === null,
                ),
            )
        : undefined;
    let open = question ?? burst;
    if (!open || open.runId !== runId) {
      mark();
      open = { kind: "peers", key: `peers:${key}`, runId, peers: [], folded: [], at };
      rows.push(open);
      burst = open;
    }
    if (folded) open.folded.push(folded);
    let held = open.peers.find((entry) => entry.peer.id === peer.id);
    if (!held) {
      held = {
        peer,
        agentId,
        sent: 0,
        received: 0,
        gist: null,
        asked: null,
        askedAt: null,
        answer: null,
        answeredAt: null,
      };
      open.peers.push(held);
    }
    if (held.gist === null && said) held.gist = gist(said);
    if (direction === "sent") {
      held.sent += 1;
      if (held.asked === null) {
        held.asked = said;
        held.askedAt = at;
      }
    } else {
      held.received += 1;
      held.answer = said;
      held.answeredAt = at;
    }
  };

  const interrupt = (row: Row) => {
    mark();
    rows.push(row);
    burst = null;
  };

  for (const message of messages) {
    const { from, to } = message;

    if (lastAt !== null && message.createdAt - lastAt >= QUIET_MS) {
      gap = { at: message.createdAt, key: message.id };
      burst = null;
    }
    lastAt = message.createdAt;

    if (message.parts.some((part) => part.type === "approval" || part.type === "question")) {
      interrupt({ kind: "message", key: message.id, message, continued: false });
      spoken = undefined;
      continue;
    }

    if (message.parts.some((part) => part.type === "decision")) {
      interrupt({ kind: "message", key: message.id, message, continued: false });
      spoken = undefined;
      continue;
    }

    if (from.kind === "agent" && to.kind === "agent") {
      count(
        toPeer(lookups.byId(from.id), from.id),
        from.id,
        "received",
        message.id,
        message.createdAt,
        message.runId,
        message.id,
        plainText(message),
      );
      spoken = undefined;
      continue;
    }

    if (from.kind === "agent" && to.kind === "system") {
      const trail: Part[] = [];
      const refused: Row[] = [];

      for (const part of message.parts) {
        const names =
          part.type === "toolCall" && part.name === "send_message"
            ? sendRecipients(part.arguments)
            : [];
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
            count(
              toPeer(card, name, name),
              card?.id ?? null,
              "sent",
              `${message.id}:${name}`,
              message.createdAt,
              message.runId,
              undefined,
              body,
            );
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

function toPerson(row: Row): row is Extract<Row, { kind: "message" }> {
  return (
    row.kind === "message" && row.message.from.kind === "agent" && row.message.to.kind === "human"
  );
}

function answers(row: Extract<Row, { kind: "message" }>): boolean {
  const { parts } = row.message;
  return parts.length > 0 && parts.every((part) => part.type === "text" || part.type === "file");
}

function joined(bursts: Extract<Row, { kind: "peers" }>[]): Extract<Row, { kind: "peers" }> | null {
  const [first, ...rest] = bursts;
  if (!first) return null;
  const peers = first.peers.map((one) => ({ ...one }));
  for (const burst of rest) {
    for (const one of burst.peers) {
      const held = peers.find((entry) => entry.peer.id === one.peer.id);
      if (!held) {
        peers.push({ ...one });
        continue;
      }
      held.sent += one.sent;
      held.received += one.received;
      held.gist ??= one.gist;
      if (held.asked === null) {
        held.asked = one.asked;
        held.askedAt = one.askedAt;
      }
      if (one.answer !== null) {
        held.answer = one.answer;
        held.answeredAt = one.answeredAt;
      }
    }
  }
  return { ...first, peers, folded: bursts.flatMap((burst) => burst.folded) };
}

export function withResults(rows: Row[], running: ReadonlySet<RunId>): Row[] {
  const last = new Map<RunId, Extract<Row, { kind: "message" }>>();
  for (const row of rows) {
    if (toPerson(row) && !running.has(row.message.runId)) last.set(row.message.runId, row);
  }
  const results = new Map<RunId, string>();
  for (const [run, row] of last) {
    if (answers(row)) results.set(run, row.key);
  }
  if (results.size === 0) return rows;

  const shown: Row[] = [];
  const held = new Map<RunId, Extract<Row, { kind: "peers" }>[]>();
  for (const row of rows) {
    if (row.kind === "peers" && results.has(row.runId) && row.peers.some((one) => one.sent > 0)) {
      held.set(row.runId, [...(held.get(row.runId) ?? []), row]);
      continue;
    }
    if (row.kind === "message" && results.get(row.message.runId) === row.key) {
      shown.push({
        kind: "result",
        key: row.key,
        message: row.message,
        handoff: joined(held.get(row.message.runId) ?? []),
      });
      results.delete(row.message.runId);
      held.delete(row.message.runId);
      continue;
    }
    shown.push(row);
  }
  return shown;
}

export function rowStandsFor(row: Row): MessageId[] {
  if (row.kind === "peers") return row.folded;
  if (row.kind === "result") return [row.message.id, ...(row.handoff?.folded ?? [])];
  if (row.kind === "refused" || row.kind === "when") return [];
  return [row.message.id];
}

export function gist(text: string): string | null {
  const line = text
    .split("\n")
    .map((part) => part.trim())
    .find((part) => part.length > 0);
  if (!line) return null;
  const plain = line.replace(/^[#>*\-\s]+/, "").replace(/\*\*/g, "");
  if (plain.length <= 72) return plain;
  const cut = plain.slice(0, 72);
  const space = cut.lastIndexOf(" ");
  return `${(space > 40 ? cut.slice(0, space) : cut).replace(/[.,;:]$/, "")}…`;
}

export function summaryLabel(summary: Pick<PeerSummary, "peer" | "sent" | "received">): string {
  const total = summary.sent + summary.received;
  if (total === 1)
    return summary.sent === 1
      ? `Messaged ${summary.peer.name}`
      : `Message from ${summary.peer.name}`;
  return `${total} messages with ${summary.peer.name}`;
}
