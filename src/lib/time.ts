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
