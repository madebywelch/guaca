import { describe, expect, it } from "vitest";

import { CHARACTERS } from "./catalog";
import { brush, DRAWN, FPS, faceAt, MOUTH_PX, POSES, poseAt, tuftOf } from "./drawn";
import { AIM, gazeAt, glance } from "./eyes";
import { bodyPoints } from "./form";
import { MOODS, type Mood } from "./moods";

const MOOD_KEYS = Object.keys(MOODS) as Mood[];

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

/** How far a point is inside the outline, less a pad. Negative is outside. */
function room(pts: [number, number][], px: number, py: number, pad: number): number {
  let near = Number.POSITIVE_INFINITY;
  for (let i = 0, j = pts.length - 1; i < pts.length; j = i++) {
    const [xi, yi] = pts[i] as [number, number];
    const [xj, yj] = pts[j] as [number, number];
    const ex = xj - xi;
    const ey = yj - yi;
    const len = ex * ex + ey * ey;
    const u = len === 0 ? 0 : Math.max(0, Math.min(1, ((px - xi) * ex + (py - yi) * ey) / len));
    near = Math.min(near, Math.hypot(px - (xi + ex * u), py - (yi + ey * u)));
  }
  return (encloses(pts, px, py) ? near : -near) - pad;
}

describe("the drawn face", () => {
  // The same gate the cut cast has, for the strokes this one adds: a brow is
  // above the eye and a mouth below it, and both have to land on the creature
  // they belong to in every mood, at every size a mouth is drawn at, wherever
  // the eyes have gone.
  it("keeps every stroke of the face inside the body", () => {
    for (const lump of CHARACTERS) {
      for (const key of MOOD_KEYS) {
        const mood = MOODS[key];
        const gazes: [number, number][] = [
          [0, -AIM.up],
          [0, AIM.down],
        ];
        for (let step = 0; step < 24; step++) gazes.push(gazeAt(step * 0.37, mood.watch?.gaze));
        gazes.forEach((gaze, step) => {
          const t = step * 0.37;
          const { pts } = bodyPoints(lump, mood.shape, t, gaze);
          const face = faceAt(lump, DRAWN[key], { t, live: true, gaze, px: 68 }, pts);
          for (const { pts: ink, pad } of face.ink) {
            for (const [x, y] of ink) {
              const clearance = room(pts, x, y, pad);
              if (!(clearance > 0)) {
                expect(clearance, `${lump.key} ${key} at ${t.toFixed(2)}s`).toBeGreaterThan(0);
              }
            }
          }
        });
      }
    }
  }, 30_000);

  // A mouth at 22px is a smudge, which is why the cut cast has none.
  it("draws a mouth only where there is room for one", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const at = { t: 0, live: false, gaze: [0, 0] as [number, number] };
    const { pts } = bodyPoints(lump, {}, 0);
    expect(faceAt(lump, DRAWN.pleased, { ...at, px: MOUTH_PX - 1 }, pts).mouth).toBe("");
    expect(faceAt(lump, DRAWN.pleased, { ...at, px: MOUTH_PX }, pts).mouth).not.toBe("");
    expect(faceAt(lump, DRAWN.surprised, { ...at, px: 24 }, pts).o).toBeNull();
    expect(faceAt(lump, DRAWN.surprised, { ...at, px: 68 }, pts).o).not.toBeNull();
  });

  // A large look is a cut hidden by a blink, the way an animator hides a head
  // turn; a glance is a snap with the eyes open.
  it("hides a large look behind shut eyes and leaves a glance open", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const watch = MOODS.idle.watch?.gaze;
    const { pts } = bodyPoints(lump, {}, 0);
    let hidden = 0;
    for (let step = 0; step < 4000; step++) {
      const t = step * 0.01;
      const g = glance(t, watch);
      const face = faceAt(
        lump,
        { ...DRAWN.idle, blink: false },
        { t, live: true, gaze: g.at, watch, px: 44 },
        pts,
      );
      if (g.size > 0.2 && g.jump < 1 / FPS) {
        expect(face.closed).toBe(true);
        hidden++;
      }
      if (g.size < 0.2) expect(face.closed).toBe(false);
    }
    expect(hidden).toBeGreaterThan(0);
  });

  it("draws one eye for a one-eyed character and two for everyone else", () => {
    for (const lump of CHARACTERS) {
      const { pts } = bodyPoints(lump, MOODS.frustrated.shape, 0);
      const face = faceAt(lump, DRAWN.frustrated, { t: 0, live: false, gaze: [0, 0], px: 44 }, pts);
      expect(face.eyes).toHaveLength(lump.eye.one ? 1 : 2);
      expect(face.brows).toHaveLength(lump.eye.one ? 1 : 2);
    }
  });
});

describe("the drawn line", () => {
  // The line is a channel of its own only if the calm moods leave it alone: a
  // rail of idle creatures whose outlines crawl is the screensaver this design
  // exists to avoid.
  it("holds a calm line still from one drawing to the next and boils a busy one", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const { pts } = bodyPoints(lump, {}, 0);
    for (const key of ["idle", "listening", "pleased", "surprised"] as const) {
      expect(brush(pts, DRAWN[key].line, 0, 0.3, 1), key).toBe(
        brush(pts, DRAWN[key].line, 7, 0.3, 1),
      );
    }
    for (const key of ["working", "frustrated"] as const) {
      expect(brush(pts, DRAWN[key].line, 0, 0.3, 1), key).not.toBe(
        brush(pts, DRAWN[key].line, 1, 0.3, 1),
      );
      expect(DRAWN[key].line.rate, key).toBe(FPS);
    }
  });

  it("draws a heavier line on a smaller creature", () => {
    const lump = CHARACTERS[0];
    if (!lump) throw new Error("no cast");
    const { pts } = bodyPoints(lump, {}, 0);
    expect(brush(pts, DRAWN.idle.line, 0, 0.3, 1.5)).not.toBe(
      brush(pts, DRAWN.idle.line, 0, 0.3, 1),
    );
  });
});

describe("the drawn timing", () => {
  // Five drawings, one of them past the new pose. A smoothstep would never
  // overshoot, and an overshoot is the whole of what pose to pose adds.
  it("changes mood in five drawings, one of them past the target", () => {
    expect(POSES).toHaveLength(5);
    expect(poseAt(0)).toBe(0);
    expect(poseAt(1.5 / FPS)).toBeGreaterThan(0);
    expect(Math.max(...POSES)).toBeGreaterThan(1);
    expect(poseAt(4 / FPS)).toBe(1);
    expect(poseAt(Number.POSITIVE_INFINITY)).toBe(1);
    // Held between drawings: the pose does not move inside one twelfth.
    expect(poseAt(2.1 / FPS)).toBe(poseAt(2.9 / FPS));
  });
});

describe("the drawn table", () => {
  it("spends the amber on exactly one mood, and it is the cut cast's one", () => {
    const amber = Object.entries(DRAWN).filter(([, face]) => face.mark === "bang");
    expect(amber.map(([key]) => key)).toEqual(["blocked"]);
    expect(MOODS.blocked.mark).toBe("bang");
  });

  // A mark nobody wears is one that could break with nothing on screen to show it.
  it("gives every character a mark for life, and uses every kind", () => {
    const kinds = new Set(CHARACTERS.map((c) => tuftOf(c.key)));
    for (const c of CHARACTERS) expect(tuftOf(c.key)).toBe(tuftOf(c.key));
    expect([...kinds].sort()).toEqual(["curl", "leaf", "none", "tick"]);
  });
});
