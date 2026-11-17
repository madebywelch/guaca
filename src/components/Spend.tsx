import type { CSSProperties } from "react";
import { useCallback, useEffect, useRef, useState } from "react";

import { PULSE_WINDOW_MS, useStore } from "../lib/store";
import type { GroupId } from "../lib/types";

const BARS = 32;

const DWELL_MS = 260;

export function money(dollars: number): string {
  if (dollars >= 100) return `$${Math.round(dollars)}`;
  if (dollars >= 1) return `$${dollars.toFixed(2)}`;
  if (dollars >= 0.01) return `$${dollars.toFixed(3)}`;
  return `$${dollars.toFixed(4)}`;
}

const MIN_PRICE = 0.0001;

export function priced(cost: number | null | undefined): cost is number {
  return cost != null && cost >= MIN_PRICE;
}

export function compact(tokens: number): string {
  if (tokens < 1000) return String(tokens);
  if (tokens < 1_000_000) {
    const thousands = tokens / 1000;
    return `${thousands < 10 ? thousands.toFixed(1) : Math.round(thousands)}k`;
  }
  const millions = tokens / 1_000_000;
  return `${millions < 10 ? millions.toFixed(1) : Math.round(millions)}M`;
}

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

interface Props {
  groupId: GroupId;
  at: { x: number; y: number };
}

export function SpendTag({ groupId, at }: Props) {
  const total = useStore((s) => s.usage[groupId]);
  const points = useStore((s) => s.pulse[groupId]);

  const [now, setNow] = useState(() => Date.now());
  const live = (points?.length ?? 0) > 0;
  useEffect(() => {
    if (!live) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [live]);

  const spent = (total?.prompt ?? 0) + (total?.completion ?? 0);
  const recent = bars(points ?? [], now);
  const peak = Math.max(...recent, 1);
  const busy = recent.slice(-3).some((value) => value > 0);

  return (
    <span
      className="spend"
      aria-hidden="true"
      data-busy={busy || undefined}
      style={{ "--tag-x": `${at.x}px`, "--tag-y": `${at.y}px` } as CSSProperties}
    >
      {total ? (
        <>
          <span className="spend__head">
            <span className="spend__total">{compact(spent)}</span>
            {priced(total.cost) && <span className="spend__cost">{money(total.cost)}</span>}
          </span>

          <span className="spend__spark">
            {recent.map((value, i) => (
              <span
                // biome-ignore lint/suspicious/noArrayIndexKey: fixed-length window
                key={i}
                className="spend__bar"
                style={{ height: `${value === 0 ? 0 : Math.max(8, (value / peak) * 100)}%` }}
              />
            ))}
          </span>

          <span className="spend__rows">
            <span className="spend__label">In</span>
            <span className="spend__figure">{total.prompt.toLocaleString()}</span>
            <span className="spend__label">Out</span>
            <span className="spend__figure">{total.completion.toLocaleString()}</span>
            <span className="spend__label">Calls</span>
            <span className="spend__figure">{total.calls.toLocaleString()}</span>
          </span>
        </>
      ) : (
        <span className="spend__quiet">Nothing spent yet.</span>
      )}
    </span>
  );
}

export interface Hovered {
  id: GroupId;
  x: number;
  y: number;
}

export function useSpendTag(): {
  shown: Hovered | null;
  show: (id: GroupId, event: { currentTarget: HTMLElement }) => void;
  hide: () => void;
} {
  const [shown, setShown] = useState<Hovered | null>(null);
  const dwell = useRef<ReturnType<typeof setTimeout>>(undefined);

  const show = useCallback((id: GroupId, event: { currentTarget: HTMLElement }) => {
    const box = event.currentTarget.getBoundingClientRect();
    clearTimeout(dwell.current);
    dwell.current = setTimeout(() => setShown({ id, x: box.left, y: box.bottom }), DWELL_MS);
  }, []);

  const hide = useCallback(() => {
    clearTimeout(dwell.current);
    setShown(null);
  }, []);

  useEffect(() => () => clearTimeout(dwell.current), []);

  return { shown, show, hide };
}
