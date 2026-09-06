import type { WorkDecision } from "./types";

/** The same row can be read, snoozed and answered. Only its durable state settles it. */
export function needsAnswer(item: WorkDecision): boolean {
  return item.status === "pending" || (item.status === "answered" && item.interrupted);
}

export function orderedDecisions(items: WorkDecision[]): WorkDecision[] {
  return [...items].sort(
    (a, b) =>
      (a.dueAt ?? Number.POSITIVE_INFINITY) - (b.dueAt ?? Number.POSITIVE_INFINITY) ||
      a.createdAt - b.createdAt ||
      a.id.localeCompare(b.id),
  );
}

export function decisionTime(at: number): string {
  return new Date(at).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
    timeZoneName: "short",
  });
}
