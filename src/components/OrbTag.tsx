import type { CSSProperties } from "react";
import { useCallback, useState } from "react";

/**
 * A crew's name, beside its circle, the moment the circle is pointed at.
 *
 * The column identifies a crew by the faces in it, and that premise is thinner
 * than it looks. The cafeteria is a copy machine with a fixed avatar and a
 * fixed color per preset, so two crews hired from the same counters draw the
 * same faces in the same colors and differ only by the few degrees of lean
 * `lib/orb.ts` takes off an agent id. Above six members every crew draws the
 * same six and a count. The operator's way out was to click through the column
 * reading names, which is the navigation the column exists to replace.
 *
 * So the name is said, and the two marks are said in words under it. Beside the
 * circle rather than under it: the column is four rem wide and a name cut to fit
 * that is a name nobody can read, which is the argument in `GroupOrb` and it
 * still holds. Laid over the app instead, this costs the column no width and the
 * layout no reflow, and it can hold a name long enough to wrap.
 *
 * This is what `title` was doing and failing at. The native tooltip waits about
 * a second, so sweeping a column of twelve shows nothing; it never appears on a
 * keyboard focus; and it is suppressed for the whole of a drag, which is the one
 * moment the circle is load-bearing, because dropping an agent on an unnamed
 * circle is how somebody is moved to the wrong crew. `title` is kept anyway, for
 * the operator who has stopped and is waiting for it.
 */
interface Props {
  /** The crew, or "All groups". */
  name: string;
  /** How big it is and what it is doing. `presenceNote`, or a bare count. */
  note: string;
  /** Middle of the circle this belongs to, in window coordinates. */
  at: number;
}

export function OrbTag({ name, note, at }: Props) {
  return (
    // Said by the button's own `aria-label` already, in the same words from the
    // same function. Drawn again in the accessibility tree it would be the crew
    // announced twice, once as a control and once as text nobody can reach.
    <span className="orb__tag" aria-hidden="true" style={{ "--tag-y": `${at}px` } as CSSProperties}>
      <span className="orb__tag-name">{name}</span>
      <span className="orb__tag-note">{note}</span>
    </span>
  );
}

/**
 * Whether a circle is being pointed at, and where it is while it is.
 *
 * The position is measured at the moment the tag opens rather than tracked,
 * because the tag is fixed to the window and the circle is inside a column that
 * scrolls: a value read once is right for exactly as long as the pointer stays
 * on the circle, and the pointer leaving is what closes it.
 *
 * Focus opens it too, and unconditionally rather than on `:focus-visible`. A
 * click focuses the circle it was already hovering, so the two agree; and the
 * pointer leaving closes what the pointer opened whether or not the button kept
 * focus, so nothing is left hanging over the app after a click.
 */
export function useOrbTag(): {
  at: number | null;
  open: (event: { currentTarget: HTMLElement }) => void;
  close: () => void;
} {
  const [at, setAt] = useState<number | null>(null);

  const open = useCallback((event: { currentTarget: HTMLElement }) => {
    const box = event.currentTarget.getBoundingClientRect();
    setAt(box.top + box.height / 2);
  }, []);

  const close = useCallback(() => setAt(null), []);

  return { at, open, close };
}
