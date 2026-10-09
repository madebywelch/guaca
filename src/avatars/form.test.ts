import { describe, expect, it } from "vitest";

import { CHARACTERS } from "./catalog";
import {
  AIM,
  cueBlink,
  type Drawn,
  type Eye,
  eyesAt,
  gazeAt,
  glance,
  LASH_REACH,
  type Mass,
  SETTLE,
  saccadeBlink,
  settle,
} from "./eyes";
import { blend, bodyPoints, FORM, grip, outline, PULL } from "./form";
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

  // A message landing is added to the look, so the gaze the body is handed can
  // be further than any gaze a mood produces, and an aimed look can land on any
  // mood at all. `PULL.hold` caps the look and the stretch is cut to the room
  // the outline has left, so this drives every creature in every mood a good
  // deal past the cap, in every direction, and expects the outline to stop
  // where it says. Before the stretch was bounded on the outline, a puddle
  // aimed down was three units over, and nothing sampled it.
  it("stays inside the reach whatever it is handed", () => {
    const past = PULL.hold * 1.15;
    let worst = 0;
    let where = "";
    for (const lump of CHARACTERS) {
      for (const key of MOOD_KEYS) {
        const mood = MOODS[key];
        for (let dir = 0; dir < 16; dir++) {
          const a = (dir / 16) * Math.PI * 2;
          const gaze: [number, number] = [Math.cos(a) * past, Math.sin(a) * past];
          for (let step = 0; step < 8; step++) {
            const t = step * 0.37;
            for (const [x, y] of bodyPoints(lump, mood.shape, t, gaze).pts) {
              const away = Math.hypot(x - FORM.center, y - FORM.center);
              if (away > worst) {
                worst = away;
                where = `${lump.key} ${key} at ${t.toFixed(2)}s looking ${dir}/16`;
              }
            }
          }
        }
      }
    }
    expect(worst, `furthest was ${where}`).toBeLessThanOrEqual(FORM.reach);
  });

  // And the cut is a cut, not a cure: a look that fits is left exactly as it
  // was asked for, so the bound changes nothing about a frame that was inside.
  it("leaves a look that fits alone", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const rest = bodyPoints(lump, {}, 0).pts;
    const look = bodyPoints(lump, {}, 0, [0.3, 0]).pts;
    const expected = 0.3 * (PULL.stretch + PULL.lean) * FORM.radius;
    const drawn = ((look[0] as number[])[0] as number) - ((rest[0] as number[])[0] as number);
    // the shear moves the same point a little; the stretch and the lean are the rest
    expect(drawn).toBeGreaterThan(expected * 0.9);
    expect(drawn).toBeLessThan(expected * 1.2);
  });

  // The whole of what tells a pear from a lump: the front narrows as it is
  // drawn out, and the back is left the shape it was.
  it("draws the front out into a snout and leaves the back round", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const rest = bodyPoints(lump, {}, 0).pts;
    const look = bodyPoints(lump, {}, 0, [0.36, 0]).pts;
    const n = FORM.samples;
    // a point a quarter turn ahead of the tip, on the way to it, is narrower
    const shoulder = Math.floor(n / 8);
    const restShoulder = (rest[shoulder] as number[])[1] as number;
    const lookShoulder = (look[shoulder] as number[])[1] as number;
    expect(lookShoulder - FORM.center).toBeLessThan(restShoulder - FORM.center);
    // and the back has moved by the lean and the shear, which is a fraction of
    // what the tip has
    const back = Math.floor(n / 2);
    const moved = ((look[back] as number[])[0] as number) - ((rest[back] as number[])[0] as number);
    const tip = ((look[0] as number[])[0] as number) - ((rest[0] as number[])[0] as number);
    expect(Math.abs(moved)).toBeLessThan(tip / 3);
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

  // The whole point of the ramp: an idle creature glancing about is a creature
  // whose body is still, and a look to the edge of the range is the one that
  // takes the body with it. Measured on the outline, not on `grip`, so a term
  // that read the raw gaze would fail here.
  it("hardly moves for a glance and moves for a stare", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const moved = (gaze: [number, number]) => {
      const rest = bodyPoints(lump, {}, 0).pts;
      const look = bodyPoints(lump, {}, 0, gaze).pts;
      let most = 0;
      for (let i = 0; i < rest.length; i++) {
        const [ax, ay] = rest[i] as [number, number];
        const [bx, by] = look[i] as [number, number];
        most = Math.max(most, Math.hypot(bx - ax, by - ay));
      }
      return most;
    };
    const glance = moved([0.12, 0]);
    const stare = moved([0.36, 0]);
    expect(glance).toBeLessThan(0.6);
    expect(stare).toBeGreaterThan(glance * 10);
  });

  it("answers no look under quiet, the whole look past wide, and none past hold", () => {
    expect(grip(PULL.quiet)).toBe(0);
    expect(grip(PULL.wide)).toBeCloseTo(PULL.wide, 6);
    expect(grip(PULL.hold * 2)).toBeCloseTo(PULL.hold, 6);
    // and it is monotone, so a longer look never moves the body less
    let last = 0;
    for (let r = 0; r <= 0.6; r += 0.01) {
      const g = grip(r);
      expect(g).toBeGreaterThanOrEqual(last);
      last = g;
    }
  });

  // A lean is the top going further than the base. The other way round is a
  // creature sliding across its box, which is the thing this design exists to
  // avoid, and a shear about the center would move both by the same amount.
  it("cranes over toward a hard look on a planted base", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const rest = bodyPoints(lump, {}, 0).pts;
    const right = bodyPoints(lump, {}, 0, [0.36, 0]).pts;
    const top = Math.floor((FORM.samples * 3) / 4);
    const base = Math.floor(FORM.samples / 4);
    const topMoved =
      ((right[top] as number[])[0] as number) - ((rest[top] as number[])[0] as number);
    const baseMoved =
      ((right[base] as number[])[0] as number) - ((rest[base] as number[])[0] as number);
    expect(topMoved).toBeGreaterThan(1);
    expect(Math.abs(baseMoved)).toBeLessThan(topMoved / 3);
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

/** The ink one drawn eye puts down, as points and how far past each it reaches. */
function inkOf(eye: Drawn): { pts: [number, number][]; pad: number }[] {
  /* The lid lines are clipped to a circle round the ball, so only the part of
     each inside it is ink; the white is ink wherever it is. */
  const inside = (pts: [number, number][]) =>
    pts.filter(([x, y]) => Math.hypot(x - eye.x, y - eye.y) <= eye.r * LASH_REACH);
  const out = [
    { pts: eye.opening, pad: 0 },
    { pts: inside(eye.lash), pad: eye.lashWidth / 2 },
  ];
  if (eye.underOpacity > 0) out.push({ pts: inside(eye.under), pad: eye.underWidth / 2 });
  return out;
}

/**
 * How much body is left around a drawn eye: the nearest edge of the outline to
 * any point of its ink, less how far the ink reaches past that point. Negative
 * is ink outside the creature it belongs to.
 */
function clearance(pts: [number, number][], eye: Drawn): number {
  let worst = Infinity;
  for (const { pts: ink, pad } of inkOf(eye)) {
    for (const [px, py] of ink) {
      let nearSquared = Infinity;
      for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
        const [xi, yi] = pts[i] as [number, number];
        const [xj, yj] = pts[j] as [number, number];
        const ex = xj - xi;
        const ey = yj - yi;
        const len = ex * ex + ey * ey;
        const u = len === 0 ? 0 : Math.max(0, Math.min(1, ((px - xi) * ex + (py - yi) * ey) / len));
        const dx = px - (xi + ex * u);
        const dy = py - (yi + ey * u);
        nearSquared = Math.min(nearSquared, dx * dx + dy * dy);
      }
      const near = Math.sqrt(nearSquared);
      worst = Math.min(worst, (encloses(pts, px, py) ? near : -near) - pad);
    }
  }
  return worst;
}

/** Area of a polygon, by the shoelace. */
function area(pts: [number, number][]): number {
  let sum = 0;
  for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
    const [xi, yi] = pts[i] as [number, number];
    const [xj, yj] = pts[j] as [number, number];
    sum += xj * yi - xi * yj;
  }
  return Math.abs(sum) / 2;
}

/** How far down the ball the white begins, in viewBox units from the ball's middle. */
function topOf(eye: Drawn): number {
  return Math.min(...eye.opening.map(([, y]) => y)) - eye.y;
}

const still = (gaze: [number, number] = [0, 0]) => ({ t: 0, live: true, gaze });
const eyeOf = (lump: (typeof CHARACTERS)[number], eye: Eye, gaze?: [number, number]) =>
  eyesAt(lump, eye, { blink: false }, still(gaze));

describe("the eyes", () => {
  // Measured against the outline itself rather than against a radius at an
  // angle: these bodies are not star-shaped, and a cloud's outer corner sits
  // over a dip between two lobes, where a radial bound is wrong in both
  // directions at once.
  it("keeps both inside the body in every mood", () => {
    for (const { lump, key, mood, t, gaze } of everyFrame()) {
      const { pts } = bodyPoints(lump, mood.shape, t, gaze);
      for (const eye of eyesAt(lump, mood.eye, mood.watch, { t, live: true, gaze })) {
        const room = clearance(pts, eye);
        expect(room, `${lump.key} ${key} at ${t.toFixed(2)}s`).toBeGreaterThan(0);
      }
    }
  });

  // An aimed look is the one gaze that does not come out of `gazeAt`, it is the
  // furthest any of them goes, and the mood it lands on at the moment a message
  // arrives is `surprised`, which has the widest eyes on the table. So it is the
  // case that decides how far a look may carry, and `everyFrame` cannot see it.
  it("keeps both inside the body while it is aimed at a peer", () => {
    for (const lump of CHARACTERS) {
      for (const key of MOOD_KEYS) {
        const mood = MOODS[key];
        for (const at of ["up", "down"] as const) {
          const gaze: [number, number] = [0, at === "up" ? -AIM.up : AIM.down];
          for (let step = 0; step < 24; step++) {
            const t = step * 0.37;
            const { pts } = bodyPoints(lump, mood.shape, t, gaze);
            for (const eye of eyesAt(lump, mood.eye, mood.watch, { t, live: true, gaze })) {
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

  // The pupil is drawn clipped to the white, so a pupil that left the ball
  // would not be seen leaving: it would be cut to a sliver and read as a
  // squint nobody asked for.
  it("keeps the pupil on the ball however far it is asked to look", () => {
    for (const lump of CHARACTERS) {
      for (const key of MOOD_KEYS) {
        for (let dir = 0; dir < 16; dir++) {
          const a = (dir / 16) * Math.PI * 2;
          const gaze: [number, number] = [Math.cos(a) * 0.6, Math.sin(a) * 0.6];
          for (const eye of eyeOf(lump, MOODS[key].eye, gaze)) {
            const off = Math.hypot(eye.pupil.x - eye.x, eye.pupil.y - eye.y);
            expect(off + eye.pupil.r * 0.8, `${lump.key} ${key} ${dir}/16`).toBeLessThanOrEqual(
              eye.r + 1e-9,
            );
          }
        }
      }
    }
  });

  // The whole reason the lids close onto each other rather than onto a line:
  // a shut eye with a sliver of white under it reads as a creature pretending.
  it("shows no white at all when it is shut", () => {
    for (const lump of CHARACTERS) {
      for (const eye of eyeOf(lump, MOODS.paused.eye)) {
        expect(area(eye.opening), lump.key).toBeLessThan(1e-6);
        expect(eye.lash.length).toBeGreaterThan(2);
        expect(eye.lashWidth).toBeGreaterThan(eyeOf(lump, MOODS.idle.eye)[0]?.lashWidth ?? 0);
      }
    }
  });

  // A blink is the same lids closing, so it can be caught halfway.
  it("blinks by closing the lids, and can be caught halfway", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const open = area((eyeOf(lump, MOODS.idle.eye)[0] as Drawn).opening);
    let least = open;
    let halfway = false;
    for (let step = 0; step < 800; step++) {
      const shut = eyesAt(lump, MOODS.idle.eye, { blink: true }, { ...still(), t: step * 0.02 })[0];
      if (!shut) throw new Error("no eye");
      const a = area(shut.opening);
      least = Math.min(least, a);
      if (a > open * 0.2 && a < open * 0.8) halfway = true;
    }
    expect(least).toBeLessThan(open * 0.05);
    expect(halfway).toBe(true);
  });

  // What an aimed look used to need a second molding for: a look down is the
  // upper lid coming down after the pupil, and it is true of every look.
  it("brings the upper lid down after the pupil", () => {
    for (const lump of CHARACTERS) {
      const level = eyeOf(lump, MOODS.idle.eye)[0] as Drawn;
      const down = eyeOf(lump, MOODS.idle.eye, [0, AIM.down])[0] as Drawn;
      expect(topOf(down), lump.key).toBeGreaterThan(topOf(level) + 0.5);
      expect(down.pupil.y - down.y, lump.key).toBeGreaterThan(0);
    }
  });

  it("drops the inner ends for cross and lifts them for worry", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    // The eye on the viewer's left: its inner end is the one on the right.
    const cross = eyeOf(lump, MOODS.frustrated.eye)[0] as Drawn;
    const worry = eyeOf(lump, MOODS.stuck.eye)[0] as Drawn;
    const ends = (eye: Drawn) => {
      const outer = eye.lash[0] as [number, number];
      const inner = eye.lash[eye.lash.length - 1] as [number, number];
      return inner[1] - outer[1];
    };
    expect(ends(cross)).toBeGreaterThan(1);
    expect(ends(worry)).toBeLessThan(-1);
  });

  // A smile with no mouth: the lower lid bows up in its middle and the white
  // over it is a crescent.
  it("smiles with the lower lid", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const eye = eyeOf(lump, MOODS.pleased.eye)[0] as Drawn;
    const middle = eye.under[Math.floor(eye.under.length / 2)] as [number, number];
    const end = eye.under[0] as [number, number];
    expect(middle[1]).toBeLessThan(end[1] - 1);
    expect(eye.underOpacity).toBeGreaterThan(0.5);
    const idle = eyeOf(lump, MOODS.idle.eye)[0] as Drawn;
    expect(area(eye.opening)).toBeLessThan(area(idle.opening) * 0.6);
  });

  // A cocked brow is the one expression a mirrored pair cannot make.
  it("lowers one lid under the other by the skew", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const [left, right] = eyeOf(lump, { ...MOODS.idle.eye, skew: 0.3 }) as [Drawn, Drawn];
    expect(topOf(right)).toBeGreaterThan(topOf(left) + 0.5);
    const [a, b] = eyeOf(lump, MOODS.idle.eye) as [Drawn, Drawn];
    expect(topOf(a)).toBeCloseTo(topOf(b), 6);
  });

  // Perspective on a turned head: the eye that went round the curve is smaller.
  it("shrinks the far eye and grows the near one on a sideways look", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const [left, right] = eyeOf(lump, MOODS.idle.eye, [0.36, 0]) as [Drawn, Drawn];
    expect(right.r).toBeGreaterThan(left.r);
    const [l0, r0] = eyeOf(lump, MOODS.idle.eye) as [Drawn, Drawn];
    expect(l0.r).toBe(r0.r);
  });

  // The pupil takes the glance, so the eyes cross the face by a fraction of a
  // glance and by more than all of a stare: the same bargain the body makes
  // through `grip`, and measured the same way, per unit of look.
  it("moves the eyes across the face for a stare and hardly for a glance", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const rest = eyeOf(lump, MOODS.idle.eye)[0] as Drawn;
    const travel = (look: number) =>
      ((eyeOf(lump, MOODS.idle.eye, [look, 0])[0] as Drawn).x - rest.x) / (look * FORM.radius);
    expect(travel(0.12)).toBeLessThan(0.7);
    expect(travel(0.36)).toBeGreaterThan(1.1);
    const glanced = eyeOf(lump, MOODS.idle.eye, [0.12, 0])[0] as Drawn;
    expect(glanced.pupil.x - glanced.x).toBeGreaterThan(0.5);
  });

  it("draws one eye for a one-eyed character and two for everyone else", () => {
    for (const lump of CHARACTERS) {
      const drawn = eyesAt(lump, MOODS.idle.eye, MOODS.idle.watch, still());
      expect(drawn).toHaveLength(lump.eye.one ? 1 : 2);
    }
  });

  // Reduced motion is a face at rest: no blink can be caught, and no shiver.
  it("holds the lids open and still when nothing may move", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const open = area((eyeOf(lump, MOODS.idle.eye)[0] as Drawn).opening);
    const held = (t: number) => ({ t, live: false, gaze: [0, 0] as [number, number], cue: 1 });
    const first = eyesAt(lump, MOODS.frustrated.eye, MOODS.frustrated.watch, held(0))[0] as Drawn;
    for (let step = 0; step < 200; step++) {
      const at = held(step * 0.05);
      const cross = eyesAt(lump, MOODS.frustrated.eye, MOODS.frustrated.watch, at)[0] as Drawn;
      const calm = eyesAt(lump, MOODS.idle.eye, MOODS.idle.watch, at)[0] as Drawn;
      expect(area(calm.opening)).toBeCloseTo(open, 6);
      expect(cross.x).toBe(first.x);
    }
  });
});

describe("the blinks a face has a reason for", () => {
  // Half of blinking is not on a timer: a face blinks into a large saccade,
  // lids leading, and not into a small one.
  it("leads a large saccade and never a small one", () => {
    const wide = { range: 0.4, hz: 0.5, cross: 0.22, far: 0.3 };
    const narrow = { range: 0.05, hz: 0.5, cross: 0.28 };
    let led = 0;
    for (let step = 0; step < 6000; step++) {
      const t = step * 0.01;
      const b = saccadeBlink(t, wide);
      if (b > 0) {
        led++;
        const g = glance(t, wide);
        expect(g.size).toBeGreaterThan(0.2);
        expect(g.jump).toBeLessThan(0.2);
      }
      expect(saccadeBlink(t, narrow)).toBe(0);
    }
    expect(led).toBeGreaterThan(0);
    expect(saccadeBlink(1, undefined)).toBe(0);
  });

  it("blinks once for a cue and then leaves the eyes alone", () => {
    expect(cueBlink(-0.01)).toBe(0);
    expect(cueBlink(0.09)).toBeGreaterThan(0.9);
    expect(cueBlink(0.2)).toBe(0);
    expect(cueBlink(Number.POSITIVE_INFINITY)).toBe(0);
  });
});

describe("the gaze", () => {
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

  // Without `far`, no target is at the edge of the range, because a uniform
  // draw does not land there; with it, some are exactly there and level.
  it("goes the whole way to one side for the share of looks it was told to", () => {
    const glances = { range: 0.4, hz: 1, cross: 0.01 };
    const looks = { ...glances, far: 0.3 };
    let farthest = 0;
    let far = 0;
    const N = 400;
    for (let i = 0; i < N; i++) {
      // sampled at the end of each slot, so every reading is a held target
      const t = i + 0.99;
      farthest = Math.max(farthest, Math.abs(gazeAt(t, glances)[0]));
      const [x, y] = gazeAt(t, looks);
      if (Math.abs(Math.abs(x) - 0.4) < 1e-9) {
        far++;
        expect(Math.abs(y)).toBeLessThan(0.4 * 0.2 + 1e-9);
      }
    }
    expect(farthest).toBeLessThan(0.4);
    expect(far / N).toBeGreaterThan(0.2);
    expect(far / N).toBeLessThan(0.4);
  });

  // Every hold the same length is a metronome, and a metronome is the thing a
  // slide is, arrived at from the other side.
  it("does not jump on the beat every time", () => {
    const gaze = { range: 0.3, hz: 1, cross: 0.05 };
    const starts: number[] = [];
    let last = gazeAt(0, gaze);
    for (let step = 1; step < 4000; step++) {
      const t = step * 0.005;
      const at = gazeAt(t, gaze);
      const still = Math.hypot(at[0] - last[0], at[1] - last[1]) < 1e-6;
      const wasStill =
        step > 1 &&
        Math.hypot(last[0] - gazeAt(t - 0.01, gaze)[0], last[1] - gazeAt(t - 0.01, gaze)[1]) < 1e-6;
      if (!still && wasStill) starts.push(t % 1);
      last = at;
    }
    expect(starts.length).toBeGreaterThan(10);
    const spread = Math.max(...starts) - Math.min(...starts);
    expect(spread).toBeGreaterThan(0.2);
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

describe("the look", () => {
  // The eyes read this too, so it must never pass its target: an eye that
  // overshoots is a wobble. And it must be quick, or a flick becomes a slide.
  it("takes a step in about the settle time and never passes it", () => {
    const mass: Mass = { gaze: [0, 0], vel: [0, 0] };
    let peak = 0;
    let arrived = Number.POSITIVE_INFINITY;
    const dt = 1 / 120;
    for (let step = 0; step < 240; step++) {
      settle(mass, [0.3, 0], dt);
      peak = Math.max(peak, mass.gaze[0]);
      if (mass.gaze[0] > 0.3 * 0.95 && arrived === Number.POSITIVE_INFINITY) arrived = step * dt;
    }
    expect(peak).toBeLessThanOrEqual(0.3 + 1e-9);
    expect(arrived).toBeLessThan(SETTLE * 2.5);
    expect(mass.gaze[0]).toBeCloseTo(0.3, 3);
  });

  // A look the whole way to one side takes longer than a glance, because the
  // body comes with it now, and a body that becomes a pear in a quarter of a
  // second is a body that snapped.
  it("crosses a long look more slowly than a short one", () => {
    const time = (script: [number, number, number][]) => {
      const gaze = { script, cross: 0.2 };
      const target = script[1] as [number, number, number];
      for (let t = 1; t < 2; t += 0.005) {
        if (Math.abs(gazeAt(t, gaze)[0] - target[0]) < 1e-3) return t - 1;
      }
      return Number.POSITIVE_INFINITY;
    };
    const glance = time([
      [0, 0, 1],
      [0.1, 0, 1],
    ]);
    const look = time([
      [0, 0, 1],
      [0.4, 0, 1],
    ]);
    expect(glance).toBeLessThan(0.25);
    expect(look).toBeGreaterThan(glance * 1.5);
    expect(look).toBeLessThan(0.45);
  });
});
