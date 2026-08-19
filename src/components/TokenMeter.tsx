import { useEffect, useState } from "react";

import { PULSE_WINDOW_MS, useStore } from "../lib/store";
import type { GroupId } from "../lib/types";

/** Bars in the sparkline. Each is one slice of {@link PULSE_WINDOW_MS}. */
const BARS = 18;

interface Props {
  groupId: GroupId;
}

/**
 * A price, at the precision the number deserves.
 *
 * Calls cost fractions of a cent, so two decimal places would show a working
 * crew as $0.00 for its first hour. The places drop away as the total grows.
 */
export function money(dollars: number): string {
  if (dollars >= 100) return `$${Math.round(dollars)}`;
  if (dollars >= 1) return `$${dollars.toFixed(2)}`;
  if (dollars >= 0.01) return `$${dollars.toFixed(3)}`;
  return `$${dollars.toFixed(4)}`;
}

/** 1.2k, 3.4M. Exact below a thousand, because early numbers are small. */
export function compact(tokens: number): string {
  if (tokens < 1000) return String(tokens);
  if (tokens < 1_000_000) {
    const thousands = tokens / 1000;
    return `${thousands < 10 ? thousands.toFixed(1) : Math.round(thousands)}k`;
  }
  const millions = tokens / 1_000_000;
  return `${millions < 10 ? millions.toFixed(1) : Math.round(millions)}M`;
}

/**
 * Buckets recent calls into bars, newest on the right.
 *
 * Relative to `now` rather than to fixed clock boundaries, so the bars drift
 * left as time passes instead of jumping a whole slot at a time.
 */
export function bars(points: { at: number; tokens: number }[], now: number): number[] {
  const slice = PULSE_WINDOW_MS / BARS;
  const out = new Array<number>(BARS).fill(0);
  for (const point of points) {
    const age = now - point.at;
    if (age < 0 || age >= PULSE_WINDOW_MS) continue;
    const index = BARS - 1 - Math.floor(age / slice);
    out[index] = (out[index] ?? 0) + point.tokens;
  }
  return out;
}

/**
 * What a group is spending, and whether anything is happening right now.
 *
 * A crew working on its own errands showed one word, "thinking", for however
 * long it took, which is indistinguishable from a crew that is stuck. A number
 * that climbs and a line that moves are the difference between watching work
 * and watching a spinner.
 */
export function TokenMeter({ groupId }: Props) {
  const total = useStore((s) => s.usage[groupId]);
  const points = useStore((s) => s.pulse[groupId]);

  // Ticks only while there is something to draw. An idle workspace does no
  // work at all, which matters when the window is open all day.
  const [now, setNow] = useState(() => Date.now());
  const live = (points?.length ?? 0) > 0;
  useEffect(() => {
    if (!live) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [live]);

  if (!total && !live) return null;

  const spent = (total?.prompt ?? 0) + (total?.completion ?? 0);
  const recent = bars(points ?? [], now);
  const peak = Math.max(...recent, 1);
  const busy = recent.slice(-3).some((value) => value > 0);

  return (
    <span
      className="meter"
      data-busy={busy || undefined}
      title={
        total
          ? [
              `${total.prompt.toLocaleString()} in, ${total.completion.toLocaleString()} out`,
              `${total.calls.toLocaleString()} model call${total.calls === 1 ? "" : "s"}`,
              // Absent on a local server, which has nothing to charge.
              total.cost === null ? null : `$${total.cost.toFixed(total.cost < 1 ? 4 : 2)}`,
            ]
              .filter(Boolean)
              .join(" · ")
          : "waiting for the first count"
      }
    >
      <span className="meter__spark" aria-hidden="true">
        {recent.map((value, i) => (
          <span
            // Position is the identity here: bar 3 is always bar 3, and its
            // height is what changes as the window slides.
            // biome-ignore lint/suspicious/noArrayIndexKey: fixed-length window
            key={i}
            className="meter__bar"
            style={{ height: `${value === 0 ? 0 : Math.max(12, (value / peak) * 100)}%` }}
          />
        ))}
      </span>
      {/* One number, not two. A group header already carries a name, a
          sparkline, a headcount and a gear, and tokens and dollars are the
          same fact twice: the price is the one an operator acts on. The count
          is what is left when a provider prices nothing, and then it is the
          only signal there is. Both are in the tooltip either way. */}
      {total?.cost != null ? (
        <span className="meter__cost">{money(total.cost)}</span>
      ) : (
        <span className="meter__count">{compact(spent)}</span>
      )}
    </span>
  );
}
