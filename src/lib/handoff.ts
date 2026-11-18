import { together } from "./running";
import { elapsed } from "./time";
import type { PeerSummary } from "./transcript";
import type { Activity, AgentId, RunId } from "./types";

export type PeerState =
  | { kind: "answered"; took: number | null }
  | { kind: "writing" | "working" | "queued" | "waitingForYou" | "asked"; since: number | null };

export interface Seen {
  activity: Record<AgentId, Activity | undefined>;
  activeRun: Record<AgentId, RunId | undefined>;
  writing: ReadonlySet<string>;
}

export function peerState(summary: PeerSummary, runId: RunId, seen: Seen): PeerState {
  if (summary.received > 0 || summary.sent === 0) {
    const took =
      summary.askedAt !== null && summary.answeredAt !== null
        ? summary.answeredAt - summary.askedAt
        : null;
    return { kind: "answered", took };
  }
  const since = summary.askedAt;
  const id = summary.agentId;
  if (id === null || seen.activeRun[id] !== runId) return { kind: "asked", since };
  if (seen.writing.has(id)) return { kind: "writing", since };
  const state = seen.activity[id]?.state;
  if (state === "awaitingApproval") return { kind: "waitingForYou", since };
  if (state === "queued") return { kind: "queued", since };
  if (state === "thinking") return { kind: "working", since };
  return { kind: "asked", since };
}

const WORD: Record<Exclude<PeerState["kind"], "answered">, string> = {
  writing: "writing",
  working: "working",
  queued: "waiting its turn",
  waitingForYou: "waiting for you",
  asked: "asked",
};

export function statusWord(state: PeerState, now: number): string {
  if (state.kind === "answered") {
    return state.took === null ? "wrote" : `answered in ${elapsed(state.took, 0)}`;
  }
  const word = WORD[state.kind];
  return state.since === null ? word : `${word}, ${elapsed(now, state.since)}`;
}

export function settled(peers: readonly PeerSummary[]): boolean {
  return peers.every((summary) => summary.sent === 0 || summary.received > 0);
}

export function foldLine(peers: readonly PeerSummary[]): string {
  const asked = peers.filter((summary) => summary.sent > 0);
  if (asked.length === 0) return `Heard from ${together(peers.map((one) => one.peer.name))}`;
  const answered = asked.filter((summary) => summary.received > 0);
  const tail =
    answered.length === asked.length
      ? asked.length === 1
        ? "answered"
        : asked.length === 2
          ? "both answered"
          : "all answered"
      : answered.length === 0
        ? asked.length === 1
          ? "no answer"
          : "no answers"
        : `${together(answered.map((one) => one.peer.name))} answered`;
  return `Asked ${together(asked.map((one) => one.peer.name))}, ${tail}`;
}

export function spanOf(peers: readonly PeerSummary[]): number | null {
  const asked = peers.flatMap((one) => (one.askedAt === null ? [] : [one.askedAt]));
  const answered = peers.flatMap((one) => (one.answeredAt === null ? [] : [one.answeredAt]));
  if (asked.length === 0 || answered.length === 0) return null;
  return Math.max(...answered) - Math.min(...asked);
}

export function waitingLine(asked: readonly string[], waiting: readonly string[]): string | null {
  if (asked.length === 0 && waiting.length === 0) return null;
  if (asked.length === 0) return `Waiting for ${together(waiting)}`;
  if (waiting.length === 0) return `Messaged ${together(asked)}`;
  return `Messaged ${together(asked)}, waiting for ${together(waiting)}`;
}
