import { describe, expect, it } from "vitest";

import { CHARACTERS } from "./catalog";
import { AIM, aimedEye, type Drawn, eyesAt, gazeAt } from "./eyes";
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

/** Whether a point is enclosed by the outline. Ray cast, so concavity counts. */
function encloses(pts: [number, number][], px: number, py: number): boolean {
  let hit = false;
  for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
    const [xi, yi] = pts[i] as [number, number];
    const [xj, yj] = pts[j] as [number, number];
    if (yi > py !== yj > py && px < ((xj - xi) * (py - yi)) / (yj - yi) + xi) hit = !hit;
  }
  return hit;
}

/**
 * How much body is left around a drawn eye: the nearest edge of the outline to
 * any corner the stroke's own hull reaches, less half the weight it is drawn
 * with. Negative is ink outside the creature it belongs to.
 */
function clearance(pts: [number, number][], eye: Drawn): number {
  const dx = Math.cos(eye.ang) * eye.w;
  const dy = Math.sin(eye.ang) * eye.w;
  const bx = -Math.sin(eye.ang) * eye.c * 2;
  const by = Math.cos(eye.ang) * eye.c * 2;
  let worst = Infinity;
  for (const [px, py] of [
    [eye.x - dx, eye.y - dy],
    [eye.x + dx, eye.y + dy],
    [eye.x + bx, eye.y + by],
  ] as [number, number][]) {
    let near = Infinity;
    for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
      const [xi, yi] = pts[i] as [number, number];
      const [xj, yj] = pts[j] as [number, number];
      const ex = xj - xi;
      const ey = yj - yi;
      const len = ex * ex + ey * ey;
      const u = len === 0 ? 0 : Math.max(0, Math.min(1, ((px - xi) * ex + (py - yi) * ey) / len));
      near = Math.min(near, Math.hypot(px - (xi + ex * u), py - (yi + ey * u)));
    }
    worst = Math.min(worst, (encloses(pts, px, py) ? near : -near) - eye.h / 2);
  }
  return worst;
}

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

  // An aimed look is the one gaze that does not come out of `gazeAt`, it is the
  // furthest any of them goes, and the mood it lands on at the moment a message
  // arrives is `surprised`, which has the widest eyes on the table. So it is the
  // case that decides how far a look may carry, and `everyFrame` cannot see it.
  //
  // Measured against the outline itself rather than against a radius at an
  // angle: these bodies are not star-shaped, and a cloud's outer corner sits
  // over a dip between two lobes, where a radial bound is wrong in both
  // directions at once.
  it("keeps both inside the body while it is aimed at a peer", () => {
    for (const lump of CHARACTERS) {
      for (const key of MOOD_KEYS) {
        const mood = MOODS[key];
        for (const at of ["up", "down"] as const) {
          const gaze: [number, number] = [0, at === "up" ? -AIM.up : AIM.down];
          for (let step = 0; step < 24; step++) {
            const t = step * 0.37;
            const { pts } = bodyPoints(lump, mood.shape, t, gaze);
            for (const eye of eyesAt(lump, aimedEye(mood.eye, at), mood.watch, t, true, gaze)) {
              const room = clearance(pts, eye);
              expect(room, `${lump.key} ${key} looking ${at} at ${t.toFixed(2)}s`).toBeGreaterThan(
                0,
              );
            }
          }
        }
      }
    }
  });

  // The whole reason an aimed look is a moulding rather than a second table of
  // faces: whatever the mood did to the eye has to survive it, or every agent
  // wears the same expression for the two seconds it is talking to somebody.
  it("moulds an aimed look into the mood rather than over it", () => {
    const angry = MOODS.frustrated.eye;
    for (const at of ["up", "down"] as const) {
      expect(aimedEye(angry, at).a).toBe(angry.a);
    }
  });

  it("drops the lid as it looks down and rounds the eye as it looks up", () => {
    // Down: lower, longer, thinner. That is a lid coming with the eyes, which
    // is the part an offset on its own cannot say.
    const dot = MOODS.idle.eye;
    const down = aimedEye(dot, "down");
    expect(down.dy as number).toBeGreaterThan(0);
    expect(down.w).toBeGreaterThan(dot.w);
    expect(down.h).toBeLessThan(dot.h);

    // Up is the starved direction, so it spends nothing: the stroke goes back
    // toward the dot it was cut from rather than fattening, and it takes no
    // offset of its own. Weight and travel are the two terms that would eat
    // what little outline sits above these eyes.
    const dash = MOODS.working.eye;
    const up = aimedEye(dash, "up");
    expect(up.w).toBeLessThan(dash.w);
    expect(up.h).toBe(dash.h);
    expect(up.dy).toBe(dash.dy);
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
