import { prefersReducedMotion } from "../lib/motion";

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
        { rootMargin: "240px" },
      );

function tick(ms: number) {
  frame = requestAnimationFrame(tick);
  if (prefersReducedMotion()) return;
  const seconds = ms / 1000;
  for (const [el, paint] of painters) {
    if (watching && !onScreen.has(el)) continue;
    paint(seconds, true);
  }
}

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

export interface Gait {
  phase: number;
  rate: number;
}

const SPREAD = { phase: 9, rate: 0.22 } as const;

function hashed(seed: string, salt: number): number {
  let h = salt >>> 0;
  for (let i = 0; i < seed.length; i++) h = (Math.imul(h, 31) + seed.charCodeAt(i)) >>> 0;
  h = (h ^ (h >>> 16)) >>> 0;
  h = Math.imul(h, 0x7feb352d) >>> 0;
  h = (h ^ (h >>> 15)) >>> 0;
  return (h % 100000) / 100000;
}

export function gaitOf(seed: string): Gait {
  return {
    phase: hashed(seed, 0) * SPREAD.phase,
    rate: 1 + (hashed(seed, 0x9e3779b1) * 2 - 1) * SPREAD.rate,
  };
}
