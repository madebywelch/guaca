import { describe, expect, it } from "vitest";

import { FORM } from "./form";
import { CREST, SILHOUETTES, type Silhouette } from "./silhouette";

const TAU = Math.PI * 2;
const KEYS = Object.keys(SILHOUETTES) as Silhouette[];
/** Fine enough that a notch or a corner cannot fall between two of them. */
const FINE = 4096;

/** The outline of one silhouette, walked at `FINE` steps. */
function walk(key: Silhouette) {
  const shape = SILHOUETTES[key];
  const pts: [number, number][] = [];
  for (let i = 0; i < FINE; i++) {
    const a = (i / FINE) * TAU;
    const r = shape(a);
    pts.push([r * Math.cos(a), r * Math.sin(a)]);
  }
  return pts;
}

function box(pts: [number, number][]) {
  let x0 = Number.POSITIVE_INFINITY;
  let x1 = Number.NEGATIVE_INFINITY;
  let y0 = Number.POSITIVE_INFINITY;
  let y1 = Number.NEGATIVE_INFINITY;
  for (const [x, y] of pts) {
    x0 = Math.min(x0, x);
    x1 = Math.max(x1, x);
    y0 = Math.min(y0, y);
    y1 = Math.max(y1, y);
  }
  return { x0, x1, y0, y1 };
}

function area(key: Silhouette): number {
  const shape = SILHOUETTES[key];
  let sum = 0;
  for (let i = 0; i < FINE; i++) {
    const r = shape((i / FINE) * TAU);
    sum += 0.5 * r * r * (TAU / FINE);
  }
  return sum;
}

function crest(key: Silhouette): number {
  let most = 0;
  for (const [x, y] of walk(key)) most = Math.max(most, Math.hypot(x, y));
  return most;
}

describe("the five", () => {
  it("answers with a real radius at every angle, including outside one turn", () => {
    for (const key of KEYS) {
      for (let i = -FINE; i < 2 * FINE; i++) {
        const r = SILHOUETTES[key]((i / FINE) * TAU);
        expect(Number.isFinite(r) && r > 0, `${key} at step ${i}`).toBe(true);
      }
    }
  });

  /* The budget the moods leave, not a taste decision. `form.test.ts` is what
     actually holds the drawing to `FORM.reach`; this is the number a new shape
     is designed against so that test does not have to be the first to say no. */
  it("keeps every silhouette inside the crest the moods can pay for", () => {
    for (const key of KEYS) expect(crest(key), key).toBeLessThanOrEqual(CREST + 1e-6);
    expect(CREST * FORM.radius).toBeLessThan(FORM.reach);
  });

  /* Sizing five shapes by eye is how a cast ends up with one member that reads
     as the small one. Equal area is the aim; the crest budget is what a shape
     with a point or a flat underside gives up to fit, and a shape that had to
     give up more than this is one to redraw rather than to scale down. */
  it("gives every silhouette the same weight, give or take what the crest costs", () => {
    for (const key of KEYS) {
      const share = area(key) / Math.PI;
      expect(share, key).toBeLessThanOrEqual(1.001);
      expect(share, key).toBeGreaterThan(0.75);
    }
    expect(area("circle") / Math.PI).toBeCloseTo(1, 3);
    expect(area("octagon") / Math.PI).toBeCloseTo(1, 3);
  });

  /* A shape drawn off center is a shape that leans in every rail it appears in,
     and nothing downstream would say so: `FORM.reach` is a radius, so an
     outline can sit low inside it and still pass. */
  it("stands every silhouette in the middle of its own box", () => {
    for (const key of KEYS) {
      const { x0, x1, y0, y1 } = box(walk(key));
      expect(Math.abs(x0 + x1), `${key} sits off to one side`).toBeLessThan(0.01);
      expect(Math.abs(y0 + y1), `${key} sits high or low`).toBeLessThan(0.01);
    }
  });

  /*
   * The outline is walked in `FORM.samples` steps, so a corner that falls
   * between two of them is a corner that gets chamfered off. A square's are at
   * 45 degrees and an octagon's at 22.5, which is why the count is divisible by
   * eight. At 28 the octagon drew as a lumpy circle and nothing failed.
   */
  it("walks the outline in enough steps to land on a corner", () => {
    expect(FORM.samples % 8).toBe(0);
    for (const key of ["square", "octagon"] as const) {
      const shape = SILHOUETTES[key];
      let sampled = 0;
      for (let i = 0; i < FORM.samples; i++) {
        sampled = Math.max(sampled, shape((i / FORM.samples) * TAU));
      }
      expect(sampled, `${key} loses its corners`).toBeCloseTo(crest(key), 6);
    }
  });

  /* The notches between the puffs are the whole of what says cloud rather than
     lump, and they are the one thing about these shapes a smaller amplitude
     would quietly remove. */
  it("notches the cloud and nothing else", () => {
    for (const key of KEYS) {
      const pts = walk(key);
      let turns = 0;
      for (let i = 0; i < pts.length; i++) {
        const a = pts[i] as [number, number];
        const b = pts[(i + 1) % pts.length] as [number, number];
        const c = pts[(i + 2) % pts.length] as [number, number];
        const cross = (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]);
        if (cross < -1e-9) turns++;
      }
      if (key === "cloud") expect(turns, "the cloud lost its notches").toBeGreaterThan(0);
      else expect(turns, `${key} is not meant to be dented`).toBe(0);
    }
  });

  /* A drop's apex and its underside are the same distance out, which is what
     keeps its crest as low as a point that sharp can be. What tells them apart
     is how fast the outline leaves: the top comes to a point and the bottom is
     a ball, so a drop that lost its cone would still pass every test above. */
  it("points the drop straight up", () => {
    const shape = SILHOUETTES.drop;
    const up = shape(-Math.PI / 2);
    const down = shape(Math.PI / 2);
    expect(up).toBeCloseTo(crest("drop"), 6);
    expect(down).toBeCloseTo(up, 6);
    expect(shape(-Math.PI / 2 + 0.2), "the point went blunt").toBeLessThan(up * 0.94);
    expect(shape(Math.PI / 2 - 0.2), "the underside came to a point").toBeGreaterThan(down * 0.99);
  });
});
