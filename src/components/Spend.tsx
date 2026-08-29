import type { CSSProperties } from "react";
import { useCallback, useEffect, useRef, useState } from "react";

import { PULSE_WINDOW_MS, useStore } from "../lib/store";
import type { GroupId } from "../lib/types";

/**
 * Bars in the sparkline. Each is one slice of {@link PULSE_WINDOW_MS}.
 *
 * Eighteen when this was a strip two and a half rem wide on the heading, where
 * more would have been a line of hairlines. The card is five times that, and at
 * eighteen the bars came out nine pixels across: a bar chart of a crew's last
 * ninety seconds, which is a claim about each bucket that a sparkline is not
 * making.
 */
const BARS = 32;

/**
 * How long the pointer has to rest on a heading before the card opens.
 *
 * A crew's heading is a band across the top of its own rows, so a pointer on
 * its way from the search box to an agent crosses one every time. Opening on
 * contact made that journey flash a panel over the row being aimed at. Long
 * enough to mean it, short enough that somebody who did mean it never waits.
 */
const DWELL_MS = 260;

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

/** The smallest price {@link money} can draw. Below it every digit is a zero. */
const MIN_PRICE = 0.0001;

/**
 * Whether there is a price worth the space it takes to draw.
 *
 * Three things report no charge and only one of them is null. A local server
 * prices nothing, so its cost is absent. A free model prices every call at a
 * real zero, and free inference over an afternoon stays zero, which drew
 * `$0.0000` beside the sparkline: seven characters of a narrow rail saying
 * nothing. A paid call small enough to round away says the same nothing at more
 * precision, which is why the floor is what {@link money} can render rather
 * than zero itself.
 *
 * Narrows, so the caller that draws the price does not have to assert it.
 */
export function priced(cost: number | null | undefined): cost is number {
  return cost != null && cost >= MIN_PRICE;
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

interface Props {
  groupId: GroupId;
  /** The bottom left corner of the heading it hangs off, in window coordinates. */
  at: { x: number; y: number };
}

/**
 * What a crew has spent, on the heading it is asked of.
 *
 * This was four glyphs and a sparkline living on the heading itself, and the
 * width was the whole problem: the readout is fixed and a crew's name is the
 * only thing on that line that can give any up, so a crew called "StopTheScam"
 * was drawn as "StopTh…" over its own agents. Nothing about the numbers earned
 * that. They are read when somebody wants to know what a crew is costing, which
 * is a question asked a few times a day, and the name is read every time the
 * eye passes the rail.
 *
 * So the heading is the name, and the numbers are what hovering it says. The
 * card is laid over the rows rather than opening a space for them, which costs
 * the rail no width and the layout no reflow, and it can hold the whole picture
 * rather than the four characters that used to fit: the total, the price, the
 * split between what was sent and what came back, and how many calls made it.
 * That is the sentence the old readout hid in a `title`, drawn instead of
 * spelled.
 *
 * It is out of the accessibility tree, and the same figures are in the crew's
 * settings behind the gear beside it, where the flow board already reports what
 * every run cost. A hover is a convenience over a number that has a home, not
 * the only way to reach one.
 */
export function SpendTag({ groupId, at }: Props) {
  const total = useStore((s) => s.usage[groupId]);
  const points = useStore((s) => s.pulse[groupId]);

  // Ticks only while there is something to draw. An idle workspace does no work
  // at all, which matters when the window is open all day.
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
    // Said nowhere in the accessibility tree, because a hover card that is also
    // a live region is a crew announcing its token count at whoever swept the
    // rail. What the numbers are for has a place of its own in the crew's
    // settings.
    <span
      className="spend"
      aria-hidden="true"
      data-busy={busy || undefined}
      style={{ "--tag-x": `${at.x}px`, "--tag-y": `${at.y}px` } as CSSProperties}
    >
      {total ? (
        <>
          <span className="spend__head">
            {/* The count leads, because it is the one that always moves: every
                call adds to it whatever the provider charges. The price joins it
                only when there is one to name, under the floor `priced` sets. */}
            <span className="spend__total">{compact(spent)}</span>
            {priced(total.cost) && <span className="spend__cost">{money(total.cost)}</span>}
          </span>

          <span className="spend__spark">
            {recent.map((value, i) => (
              <span
                // Position is the identity here: bar 3 is always bar 3, and its
                // height is what changes as the window slides.
                // biome-ignore lint/suspicious/noArrayIndexKey: fixed-length window
                key={i}
                className="spend__bar"
                style={{ height: `${value === 0 ? 0 : Math.max(8, (value / peak) * 100)}%` }}
              />
            ))}
          </span>

          {/* Exact rather than compact, which is the point of opening it. The
              headline is the figure you can read at a glance and this is the one
              you came to check, so rounding it here would leave the card saying
              the same thing twice. */}
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
        // Rather than refusing to open. A card that appears for the crews that
        // have spent something and not for the ones that have not is a hover
        // nobody can tell from a broken one.
        <span className="spend__quiet">Nothing spent yet.</span>
      )}
    </span>
  );
}

/** The heading being pointed at, and the corner its card hangs off. */
export interface Hovered {
  id: GroupId;
  x: number;
  y: number;
}

/**
 * Which heading is being asked, and where it was when it was asked.
 *
 * The position is measured when the card opens rather than tracked, on the same
 * argument `useOrbTag` makes: the card is fixed to the window and the heading is
 * inside a list that scrolls, so a value read once is right for exactly as long
 * as the pointer stays on the heading, and the pointer leaving is what closes
 * it.
 *
 * One hook for every heading in the rail, because there is one pointer. A crew
 * asking for its own card would leave a card open behind whichever heading the
 * pointer left in a hurry.
 */
export function useSpendTag(): {
  shown: Hovered | null;
  show: (id: GroupId, event: { currentTarget: HTMLElement }) => void;
  hide: () => void;
} {
  const [shown, setShown] = useState<Hovered | null>(null);
  const dwell = useRef<ReturnType<typeof setTimeout>>(undefined);

  const show = useCallback((id: GroupId, event: { currentTarget: HTMLElement }) => {
    // Read now rather than inside the timer: `currentTarget` is null by the time
    // React is done with the event, and the heading has not moved in the
    // quarter second either way.
    const box = event.currentTarget.getBoundingClientRect();
    clearTimeout(dwell.current);
    dwell.current = setTimeout(() => setShown({ id, x: box.left, y: box.bottom }), DWELL_MS);
  }, []);

  const hide = useCallback(() => {
    clearTimeout(dwell.current);
    setShown(null);
  }, []);

  // A rail that unmounts mid-dwell would otherwise open a card over whatever
  // replaced it.
  useEffect(() => () => clearTimeout(dwell.current), []);

  return { shown, show, hide };
}
