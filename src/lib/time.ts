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

export type Unit = "minutes" | "hours" | "days";

const SECONDS: Record<Unit, number> = { minutes: 60, hours: 3600, days: 86_400 };

/** The shortest true way to say a gap: 7200 seconds is "2 hours". */
export function splitGap(secs: number): { value: number; unit: Unit } {
  for (const unit of ["days", "hours", "minutes"] as Unit[]) {
    const size = SECONDS[unit];
    if (secs % size === 0 && secs >= size) return { value: secs / size, unit };
  }
  // Under a minute cannot be set in the editor at all, and the backend refuses
  // it, but a row written by something else still has to draw.
  return { value: Math.max(1, Math.round(secs / 60)), unit: "minutes" };
}

export function toSeconds(value: number, unit: Unit): number {
  return Math.max(1, Math.round(value)) * SECONDS[unit];
}

/** How often a routine fires, said the way a person would. */
export function cadence(everySecs: number | null): string {
  if (everySecs === null) return "once";
  const { value, unit } = splitGap(everySecs);
  return value === 1 ? `every ${unit.slice(0, -1)}` : `every ${value} ${unit}`;
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
