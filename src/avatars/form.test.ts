import { describe, expect, it } from "vitest";

import { CHARACTERS } from "./catalog";
import { eyesAt, gazeAt } from "./eyes";
import { blend, bodyPoints, FORM, outline } from "./form";
import { MOODS, type Mood } from "./moods";

const MOOD_KEYS = Object.keys(MOODS) as Mood[];

/**
 * Every frame of every creature in every mood, coarsely.
 *
 * The old catalog checked bounding boxes of hand-written paths, because a
 * character was a path somebody drew. Nothing is drawn now, so the thing worth
 * checking is different and stronger: no combination of identity, mood, phase
 * and gaze may put ink outside the box the rest of the app sizes against.
 */
function* everyFrame() {
  for (const lump of CHARACTERS) {
    for (const key of MOOD_KEYS) {
      const mood = MOODS[key];
      for (let step = 0; step < 24; step++) {
        const t = step * 0.37;
        const gaze = gazeAt(t, mood.watch?.gaze);
        yield { lump, key, mood, t, gaze };
      }
    }
  }
}

describe("the body", () => {
  it("never draws outside the reach the rest of the app sizes against", () => {
    let worst = 0;
    let where = "";
    for (const { lump, key, mood, t, gaze } of everyFrame()) {
      for (const [x, y] of bodyPoints(lump, mood.shape, t, gaze).pts) {
        const away = Math.hypot(x - FORM.center, y - FORM.center);
        if (away > worst) {
          worst = away;
          where = `${lump.key} ${key} at ${t.toFixed(2)}s`;
        }
      }
    }
    expect(worst, `furthest was ${where}`).toBeLessThanOrEqual(FORM.reach);
  });

  // The number is not decoration: `orb.test.ts` seats a crew on it, so a reach
  // that quietly fell would leave every group's circle drawn too loose.
  it("actually uses the reach it claims", () => {
    let worst = 0;
    for (const { lump, mood, t, gaze } of everyFrame()) {
      for (const [x, y] of bodyPoints(lump, mood.shape, t, gaze).pts) {
        worst = Math.max(worst, Math.hypot(x - FORM.center, y - FORM.center));
      }
    }
    expect(worst).toBeGreaterThan(FORM.reach * 0.85);
  });

  it("stays a closed loop of the same length whatever happens to it", () => {
    for (const { lump, mood, t, gaze } of everyFrame()) {
      const { pts } = bodyPoints(lump, mood.shape, t, gaze);
      expect(pts).toHaveLength(FORM.samples);
      for (const [x, y] of pts) {
        expect(Number.isFinite(x) && Number.isFinite(y)).toBe(true);
      }
    }
  });

  // A gaze the eyes have not taken yet must leave the resting shape alone, or
  // every creature on screen is permanently deformed by a look nobody took.
  it("is its resting self when it is looking straight ahead", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const still = bodyPoints(lump, {}, 0, [0, 0]).pts;
    const same = bodyPoints(lump, {}, 0).pts;
    expect(still).toEqual(same);
  });

  it("swells toward a look and flattens behind it", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const rest = bodyPoints(lump, {}, 0).pts;
    const right = bodyPoints(lump, {}, 0, [0.3, 0]).pts;
    const near = 0;
    const far = Math.floor(FORM.samples / 2);
    expect((right[near] as number[])[0]).toBeGreaterThan((rest[near] as number[])[0] as number);
    expect((right[far] as number[])[0]).toBeGreaterThan((rest[far] as number[])[0] as number);
  });

  it("blends one shape into another without leaving either", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const a = bodyPoints(lump, MOODS.idle.shape, 0).pts;
    const b = bodyPoints(lump, MOODS.stuck.shape, 0).pts;
    expect(blend(a, b, 0)).toEqual(a);
    expect(blend(a, b, 1)).toEqual(b);
    const half = blend(a, b, 0.5);
    for (let i = 0; i < a.length; i++) {
      const lo = Math.min((a[i] as number[])[1] as number, (b[i] as number[])[1] as number);
      const hi = Math.max((a[i] as number[])[1] as number, (b[i] as number[])[1] as number);
      expect((half[i] as number[])[1]).toBeGreaterThanOrEqual(lo);
      expect((half[i] as number[])[1]).toBeLessThanOrEqual(hi);
    }
  });

  it("writes a closed path of cubics and nothing else", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const d = outline(bodyPoints(lump, MOODS.idle.shape, 0).pts);
    expect(d.startsWith("M")).toBe(true);
    expect(d.match(/C/g) ?? []).toHaveLength(FORM.samples);
    expect(d).not.toMatch(/NaN|Infinity/);
  });
});

/** How far the outline is from the center in one direction. */
function edgeAt(pts: [number, number][], angle: number): number {
  const at = ((angle / (Math.PI * 2)) * pts.length + pts.length) % pts.length;
  const a = pts[Math.floor(at) % pts.length] as [number, number];
  const b = pts[(Math.floor(at) + 1) % pts.length] as [number, number];
  const u = at - Math.floor(at);
  const ra = Math.hypot(a[0] - FORM.center, a[1] - FORM.center);
  const rb = Math.hypot(b[0] - FORM.center, b[1] - FORM.center);
  return ra + (rb - ra) * u;
}

describe("the eyes", () => {
  it("keeps both inside the body in every mood", () => {
    for (const { lump, key, mood, t, gaze } of everyFrame()) {
      const { pts } = bodyPoints(lump, mood.shape, t, gaze);
      for (const eye of eyesAt(lump, mood.eye, mood.watch, t, true, gaze)) {
        const dx = eye.x - FORM.center;
        const dy = eye.y - FORM.center;
        const away = Math.hypot(dx, dy) + eye.w + eye.h / 2;
        const edge = edgeAt(pts, Math.atan2(dy, dx));
        expect(away, `${lump.key} ${key} at ${t.toFixed(2)}s`).toBeLessThan(edge);
      }
    }
  });

  it("draws one eye for a one-eyed character and two for everyone else", () => {
    for (const lump of CHARACTERS) {
      const drawn = eyesAt(lump, MOODS.idle.eye, MOODS.idle.watch, 0, true, [0, 0]);
      expect(drawn).toHaveLength(lump.eye.one ? 1 : 2);
    }
  });

  // The whole reason there is one primitive rather than a set of faces: a blink
  // is the dot being moulded into a line, so it can be caught halfway.
  it("moulds a dot into a line and back", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const open = eyesAt(lump, { w: 0.02, h: 2 }, { blink: false }, 0, true, [0, 0])[0];
    if (!open) throw new Error("no eye");
    let widest = open;
    for (let step = 0; step < 400; step++) {
      const shut = eyesAt(lump, { w: 0.02, h: 2 }, { blink: true }, step * 0.03, true, [0, 0])[0];
      if (shut && shut.w > widest.w) widest = shut;
    }
    expect(widest.w).toBeGreaterThan(open.w * 8);
    expect(widest.h).toBeLessThan(open.h * 0.5);
  });

  it("holds a gaze still between jumps rather than sliding it", () => {
    const gaze = { range: 0.3, hz: 0.5, cross: 0.2 };
    let moving = 0;
    let last = gazeAt(0, gaze);
    for (let step = 1; step < 200; step++) {
      const at = gazeAt(step * 0.02, gaze);
      if (Math.hypot(at[0] - last[0], at[1] - last[1]) > 1e-4) moving++;
      last = at;
    }
    // A saccade crosses in 0.2s of every 2s, so at most a fifth of the frames.
    expect(moving).toBeLessThan(200 * 0.25);
  });

  it("follows a written gaze in the order it was written", () => {
    const script: [number, number, number][] = [
      [0, 0, 1],
      [0.3, -0.3, 1],
    ];
    const gaze = { script, cross: 0.1 };
    expect(gazeAt(0.9, gaze)[1]).toBeCloseTo(0, 2);
    expect(gazeAt(1.5, gaze)[1]).toBeCloseTo(-0.3, 2);
    // and it cycles rather than running out
    expect(gazeAt(2.9, gaze)[1]).toBeCloseTo(0, 2);
  });
});
