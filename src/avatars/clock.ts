/**
 * One frame loop for every creature on screen.
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
