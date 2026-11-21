import { api } from "./ipc";
import { whoIsWaiting } from "./questions";
import { relativeTime } from "./time";
import type { AgentId, Group, GroupId, Outcome, OutcomeStatus, RunId } from "./types";

export const EARLIER_MS = 7 * 24 * 3_600_000;

export const UNSEEN_LIMIT = 200;

export const EARLIER_LIMIT = 100;

export const PANEL_LINES = 5;

export function told(outcome: Outcome): boolean {
  return outcome.status !== "nothingNew";
}

function newestFirst(a: Outcome, b: Outcome): number {
  return b.finishedAt - a.finishedAt || (a.runId < b.runId ? 1 : a.runId > b.runId ? -1 : 0);
}

export function finished(outcomes: Outcome[]): Outcome[] {
  return outcomes.filter((one) => one.seenAt === null && told(one)).sort(newestFirst);
}

export function earlier(outcomes: Outcome[], now: number): Outcome[] {
  return outcomes
    .filter((one) => one.seenAt !== null && told(one) && one.finishedAt >= now - EARLIER_MS)
    .sort(newestFirst);
}

export function didNotFinish(outcome: Outcome): boolean {
  return outcome.status === "failed" || outcome.status === "interrupted";
}

export const NOT_SENT_AGAIN =
  "This task can no longer be sent again from here. Open the conversation and write it again.";

export async function retryOutcome(outcome: Outcome): Promise<RunId> {
  const record = outcome.noteId ? await api.taskRecord(outcome.agentId, outcome.runId) : null;
  const notice = record?.messages?.find((message) => message.id === outcome.noteId);
  if (!notice?.cause) throw new Error(NOT_SENT_AGAIN);
  return api.retryTurn(notice.channelId, notice.cause);
}

export function statusWord(status: OutcomeStatus): string | null {
  switch (status) {
    case "failed":
      return "failed";
    case "interrupted":
      return "interrupted";
    case "stopped":
      return "stopped";
    case "needsYou":
      return "needs you";
    default:
      return null;
  }
}

export function railWord(status: OutcomeStatus): string {
  switch (status) {
    case "failed":
    case "interrupted":
    case "stopped":
      return status;
    default:
      return "done";
  }
}

export function railMeta(outcome: Outcome, now: number): string {
  return `${railWord(outcome.status)} ${relativeTime(outcome.finishedAt, now)}`;
}

export function newestByAgent(outcomes: Outcome[]): Record<AgentId, Outcome> {
  const newest: Record<AgentId, Outcome> = {};
  for (const one of finished(outcomes)) {
    if (!newest[one.agentId]) newest[one.agentId] = one;
  }
  return newest;
}

export interface Shelf {
  group: { id: GroupId; name: string } | null;
  outcomes: Outcome[];
}

export function shelves(outcomes: Outcome[], groups: Group[]): Shelf[] {
  const sorted = [...outcomes].sort(newestFirst);
  const order: GroupId[] = [];
  for (const one of sorted) if (!order.includes(one.groupId)) order.push(one.groupId);
  if (order.length < 2) return sorted.length === 0 ? [] : [{ group: null, outcomes: sorted }];
  return order.map((id) => ({
    group: { id, name: groups.find((group) => group.id === id)?.name ?? "A deleted group" },
    outcomes: sorted.filter((one) => one.groupId === id),
  }));
}

export function drawnThrough(outcomes: Outcome[]): number | null {
  return outcomes.reduce<number | null>(
    (newest, one) => (newest === null || one.finishedAt > newest ? one.finishedAt : newest),
    null,
  );
}

export function moreLine(total: number, shown: number): string | null {
  const rest = total - shown;
  return rest > 0 ? `And ${rest} more in For you` : null;
}

export function forYouSummary(waiting: string[], unseen: number, stalled = 0): string {
  if (waiting.length > 0) return `${whoIsWaiting(waiting)}.`;
  const since = [
    ...(unseen > 0 ? [`${unseen} finished`] : []),
    ...(stalled > 0 ? [`${stalled} did not finish`] : []),
  ];
  if (since.length > 0)
    return `Nothing needs your answer. ${since.join(" and ")} since you last looked.`;
  return "Nothing needs your answer right now.";
}
