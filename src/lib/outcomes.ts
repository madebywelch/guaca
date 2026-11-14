/**
 * Finished work, as For you, the rail and the menu bar draw it.
 *
 * The host writes one outcome per finished run and says when the operator has
 * seen it (`db/outcomes.rs`). Everything here is what the window decides about
 * that list: which outcomes are news, in what order, under which heading and in
 * which words. No DOM, so the suite beside this file is where those decisions
 * are argued, and the three surfaces cannot word one run three ways.
 *
 * Finished sits under what needs the operator and never competes with it. A
 * finished run is something to read when there is time, so nothing here is
 * drawn in the attention color or counted on the icon.
 */

import { relativeTime } from "./time";
import type { AgentId, Group, GroupId, Outcome, OutcomeStatus } from "./types";

/** How far back Earlier reaches. */
export const EARLIER_MS = 7 * 24 * 3_600_000;

/**
 * How many unseen outcomes a window reads. Past this the operator has stopped
 * reading the list one line at a time, and Mark all seen is what they want.
 */
export const UNSEEN_LIMIT = 200;

/** How many outcomes Earlier reads. */
export const EARLIER_LIMIT = 100;

/** How many lines the menu bar panel draws before it points at For you. */
export const PANEL_LINES = 5;

/**
 * Whether an outcome is news at all.
 *
 * A recurring check that found nothing is not. Starting quiet means it says
 * nothing anywhere: not in Finished, not on the rail, not in the count.
 */
export function told(outcome: Outcome): boolean {
  return outcome.status !== "nothingNew";
}

/** Newest first, and by run for two that finished in the same millisecond. */
function newestFirst(a: Outcome, b: Outcome): number {
  return b.finishedAt - a.finishedAt || (a.runId < b.runId ? 1 : a.runId > b.runId ? -1 : 0);
}

/** What Finished lists: unseen news, newest first. */
export function finished(outcomes: Outcome[]): Outcome[] {
  return outcomes.filter((one) => one.seenAt === null && told(one)).sort(newestFirst);
}

/**
 * What Earlier lists: news the operator has seen, from the last seven days,
 * newest first. Unseen ones are Finished's, so one run is never in both.
 */
export function earlier(outcomes: Outcome[], now: number): Outcome[] {
  return outcomes
    .filter((one) => one.seenAt !== null && told(one) && one.finishedAt >= now - EARLIER_MS)
    .sort(newestFirst);
}

/**
 * The word beside a headline, or nothing for a run that simply finished.
 *
 * Only an ending worth knowing before reading the line gets one. "done" on
 * every line would be a column of the same word.
 */
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

/**
 * The word on a rail row, before the age.
 *
 * A run that left a decision is "done" here, not "needs you": on the rail that
 * phrase means a turn parked on the operator, drawn in the attention color, and
 * the decision itself is already counted in For you.
 */
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

/** The rail's meta for an agent with news: "done 5h". */
export function railMeta(outcome: Outcome, now: number): string {
  return `${railWord(outcome.status)} ${relativeTime(outcome.finishedAt, now)}`;
}

/** Each agent's newest unseen news, which is the one its rail row speaks for. */
export function newestByAgent(outcomes: Outcome[]): Record<AgentId, Outcome> {
  const newest: Record<AgentId, Outcome> = {};
  for (const one of finished(outcomes)) {
    if (!newest[one.agentId]) newest[one.agentId] = one;
  }
  return newest;
}

/** One heading's worth of a list. `group` is null when no heading is drawn. */
export interface Shelf {
  group: { id: GroupId; name: string } | null;
  outcomes: Outcome[];
}

/**
 * A list of outcomes, under a heading per group when it holds more than one.
 *
 * A heading that is the only heading distinguishes nobody, which is the rule
 * the groups' column and the menu bar's tooltip are drawn by. The group with
 * the newest line comes first, so the list still reads newest first from the
 * top.
 */
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

/**
 * The bound for Mark all seen: when the newest line drawn finished.
 *
 * A time rather than "all", because a run that finishes while the list is open
 * was not on screen when the button was pressed.
 */
export function drawnThrough(outcomes: Outcome[]): number | null {
  return outcomes.reduce<number | null>(
    (newest, one) => (newest === null || one.finishedAt > newest ? one.finishedAt : newest),
    null,
  );
}

/** What is left past the lines the panel draws, as a line of its own. */
export function moreLine(total: number, shown: number): string | null {
  const rest = total - shown;
  return rest > 0 ? `And ${rest} more in For you` : null;
}

/**
 * The sentence under For you's title.
 *
 * What needs the operator leads, and finished work is named only when nothing
 * does, so it never reads as one more thing to answer.
 */
export function forYouSummary(needs: number, unseen: number): string {
  if (needs > 0) return `${needs} ${needs === 1 ? "item needs" : "items need"} your attention.`;
  if (unseen > 0) return `Nothing needs your answer. ${unseen} finished since you last looked.`;
  return "Nothing needs your answer right now.";
}
