import { useEffect, useState } from "react";

/**
 * Compact relative time, e.g. `now`, `4m`, `3h`, `2d`.
 *
 * Deliberately short: it sits in a narrow sidebar column beside an agent's
 * name, where "4 minutes ago" would push the name into an ellipsis.
 */
export function relativeTime(at: number, now: number): string {
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  if (seconds < 45) return "now";

  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${Math.max(1, minutes)}m`;

  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h`;

  const days = Math.round(hours / 24);
  if (days < 7) return `${days}d`;

  return `${Math.round(days / 7)}w`;
}

/**
 * How long something has been going, counted in seconds.
 *
 * Not {@link relativeTime}, which rounds anything under three quarters of a
 * minute to "now". This is read while waiting on a call that has not come back,
 * where the whole question is whether the number is still moving, and "now" for
 * the first forty-four seconds of a wait answers it wrong. Whether a wait is
 * long enough to be worth reporting at all is the caller's decision, not this
 * one's.
 */
export function elapsed(now: number, since: number): string {
  const seconds = Math.max(0, Math.floor((now - since) / 1000));
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, "0")}s`;
}

/** Local midnight, so a day is a day and not a fixed number of milliseconds. */
function midnight(at: number): number {
  const day = new Date(at);
  day.setHours(0, 0, 0, 0);
  return day.getTime();
}

/**
 * When a stretch of conversation picked up again, for the line between two of
 * them.
 *
 * Named while a name is unambiguous and dated once it is not: "Tuesday" means
 * this week, and three weeks back it means nothing at all. Rounded from two
 * local midnights rather than divided out of a difference, because a day is 23
 * or 25 hours twice a year.
 */
export function whenLabel(at: number, now: number): string {
  // A leading zero is for a column of times that have to line up. This is one
  // time in a sentence, so it is written the way it is said.
  const clock = new Date(at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const days = Math.round((midnight(now) - midnight(at)) / 86_400_000);

  if (days <= 0) return `Today ${clock}`;
  if (days === 1) return `Yesterday ${clock}`;
  if (days < 7) return `${new Date(at).toLocaleDateString([], { weekday: "long" })} ${clock}`;
  return `${new Date(at).toLocaleDateString([], { day: "numeric", month: "short" })} ${clock}`;
}

/**
 * A clock that ticks slowly, so relative labels stay honest without
 * re-rendering the rail every frame.
 */
export function useNow(intervalMs = 30_000): number {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);

  return now;
}
