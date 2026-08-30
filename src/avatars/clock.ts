/**
 * The clock every creature shares, and the one each of them keeps.
 *
 * A rail of a dozen agents is a dozen avatars, and a dozen `requestAnimationFrame`
 * loops is a dozen chances for one of them to be left running after its row is
 * gone. There is one loop here, it runs only while something is registered, and
 * what is off screen is not computed at all: a transcript with sixty faces in it
 * is otherwise sixty outlines a frame for the sake of the eight you can see.
 *
 * Nothing about correctness depends on this. Every avatar paints itself once on
 * mount and again whenever its props change, so an operator who asked for
 * reduced motion, a hidden window and a row scrolled out of view all draw the
 * right thing; the loop only makes it move.
 *
 * One clock for every creature is also what makes a rail read as one animal,
 * which is what `gaitOf` is for: a mood is a table of rates shared by everybody
 * in it, so the shared seconds have to be bent per creature before they are
 * spent. That is the bottom of this file.
 */

import { prefersReducedMotion } from "../lib/motion";

/**
 * `live` is whether motion is wanted at all. Asked once a frame here rather
 * than once a frame per creature: `matchMedia` is not free and the answer is
 * the same for every face on screen.
 */
export type Painter = (seconds: number, live: boolean) => void;

const painters = new Map<Element, Painter>();
const onScreen = new WeakSet<Element>();
let frame = 0;

const watching =
  typeof IntersectionObserver === "undefined"
    ? null
    : new IntersectionObserver(
        (rows) => {
          for (const row of rows) {
            if (row.isIntersecting) onScreen.add(row.target);
            else onScreen.delete(row.target);
          }
        },
        /* Generous, so a row scrolled to is already moving by the time it
           arrives rather than starting its breath under the operator's eye. */
        { rootMargin: "240px" },
      );

function tick(ms: number) {
  /* Scheduled before the work, so one painter throwing does not stop the rest
     of the app's faces for the life of the session. */
  frame = requestAnimationFrame(tick);
  /* Read here rather than held, so an operator who changes the setting while
     the app is running gets the next frame right. A still creature is still
     drawn: it was painted on mount and is repainted whenever anything about it
     changes, so nothing depends on this loop for being correct. */
  if (prefersReducedMotion()) return;
  const seconds = ms / 1000;
  for (const [el, paint] of painters) {
    if (watching && !onScreen.has(el)) continue;
    paint(seconds, true);
  }
}

/** Registers a painter until the returned function is called. */
export function join(el: Element, paint: Painter): () => void {
  painters.set(el, paint);
  if (watching) watching.observe(el);
  else onScreen.add(el);
  if (!frame) frame = requestAnimationFrame(tick);

  return () => {
    painters.delete(el);
    watching?.unobserve(el);
    onScreen.delete(el);
    if (painters.size === 0 && frame) {
      cancelAnimationFrame(frame);
      frame = 0;
    }
  };
}

/* --- the clock one creature keeps ---------------------------------------- */

/**
 * How one creature's own seconds run.
 *
 * A mood is a table of rates every agent in it shares, so eight idle agents on
 * one clock breathe as one animal. An offset alone does not fix that: it moves
 * a creature's cycle without changing its tempo, and two creatures at one tempo
 * hold whatever gap they started with forever, which is the thing that reads as
 * choreography rather than as life. So a seed buys two numbers, and the second
 * is the one that does the work: at this spread a pair drifts half a cycle
 * apart inside about fifteen seconds of idling, and keeps going.
 *
 * Only cycles are on this clock. Anything measuring the age of an event -- a
 * mood becoming another, a message landing, a turn finishing -- stays on the
 * shared seconds, or a creature that breathes quickly would also settle
 * quickly, and how fast a face reacts would be a property of its id.
 */
export interface Gait {
  /** Seconds added to the shared clock, so two creatures start apart. */
  phase: number;
  /** And multiplied into it, so they do not stay the same distance apart. */
  rate: number;
}

/**
 * How far apart two creatures are allowed to be: seconds of phase, parts of
 * tempo. The tempo is the deliberate one. Much under this and a row still
 * pulses together; much over and the same mood reads as two different amounts
 * of urgency, which is a thing `moods.ts` is supposed to be the only source of.
 */
const SPREAD = { phase: 9, rate: 0.22 } as const;

/**
 * Deterministic 0..1 from a string and a salt. The avalanche is not decoration:
 * two agent ids that differ in one character have to land on unrelated tempos,
 * and a plain multiply-add leaves them adjacent.
 */
function hashed(seed: string, salt: number): number {
  let h = salt >>> 0;
  for (let i = 0; i < seed.length; i++) h = (Math.imul(h, 31) + seed.charCodeAt(i)) >>> 0;
  /* Every step is put back into 32 unsigned bits, because `^` in JavaScript
     hands back a signed integer: one uncoerced xor here and a creature runs at
     a negative rate, which is a face playing backwards. */
  h = (h ^ (h >>> 16)) >>> 0;
  h = Math.imul(h, 0x7feb352d) >>> 0;
  h = (h ^ (h >>> 15)) >>> 0;
  return (h % 100000) / 100000;
}

/** The gait of whoever this seed names. The same agent moves the same way every reload. */
export function gaitOf(seed: string): Gait {
  return {
    phase: hashed(seed, 0) * SPREAD.phase,
    rate: 1 + (hashed(seed, 0x9e3779b1) * 2 - 1) * SPREAD.rate,
  };
}
