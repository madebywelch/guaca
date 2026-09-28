import { describe, expect, it } from "vitest";

import { brandAt, HOLD, type Scene, SETTLED, type Situation, toward } from "./brand";

const QUIET: Situation = { hoverAt: null, waiting: false, still: false };
const at = (t: number, s: Partial<Situation> = {}) => brandAt(t, { ...QUIET, ...s });
const scene = ({ next: _, ...rest }: Scene) => rest;

describe("reduced motion", () => {
  it("never performs: both there, at rest, and nothing scheduled", () => {
    const s = at(0, { still: true });
    expect(s.left).toEqual({ shown: true, mood: "idle", gaze: null, gesture: null });
    expect(s.right).toEqual(s.left);
    expect(s.named).toBe(false);
    expect(s.next).toBe(Number.POSITIVE_INFINITY);
  });

  it("shows the name for a hover and holds no face it would have to take back", () => {
    const s = at(2, { still: true, hoverAt: 1.9 });
    expect(s.named).toBe(true);
    expect(s.left.gaze).toBe("pointer");
    expect(s.left.mood).toBe("idle");
    expect(s.next).toBe(Number.POSITIVE_INFINITY);
  });
});

describe("a turn parked on the operator", () => {
  it("is the first creature's blocked face, with its own look up at the badge", () => {
    const s = at(SETTLED + 2, { waiting: true });
    expect(s.left.mood).toBe("blocked");
    expect(s.left.gaze).toBeNull();
    expect(s.right.mood).toBe("idle");
  });

  it("waits for the performance to end rather than breaking into it", () => {
    expect(at(1, { waiting: true })).toEqual(at(1));
    expect(at(SETTLED, { waiting: true }).left.mood).toBe("blocked");
  });

  it("keeps the face while the pointer is on it", () => {
    const s = at(SETTLED + 2, { waiting: true, hoverAt: SETTLED + 1.9 });
    expect(s.left.mood).toBe("blocked");
    expect(s.left.gaze).toBe("pointer");
    expect(s.right.mood).toBe("pleased");
  });
});

describe("the launch", () => {
  it("brings them in one at a time", () => {
    expect(at(0).left.shown).toBe(false);
    expect(at(0.1).left.shown).toBe(true);
    expect(at(0.1).right.shown).toBe(false);
    expect(at(0.5).right.shown).toBe(true);
  });

  it("has them look at each other, then throw and catch", () => {
    const looking = at(1);
    expect((looking.left.gaze as number[])[0]).toBeGreaterThan(0);
    expect((looking.right.gaze as number[])[0]).toBeLessThan(0);
    expect(at(1.2).left.gesture).toBeNull();
    expect(at(1.3).left.gesture).toBe("send");
    expect(at(1.3).right.gesture).toBeNull();
    expect(at(1.6).right.gesture).toBe("receive");
    expect(at(1.6).right.mood).toBe("surprised");
    // Null either side, so the avatar sees each throw as a change.
    expect(at(2.2).left.gesture).toBeNull();
  });

  it("writes the name in after the catch, and turns them both to the operator", () => {
    expect(at(1.9).named).toBe(false);
    expect(at(2).named).toBe(true);
    const turned = at(3);
    expect(turned.left.gaze).toEqual(turned.right.gaze);
    expect(turned.left.mood).toBe("pleased");
    expect(turned.right.mood).toBe("pleased");
  });

  it("folds the name away at the hold and leaves the pair", () => {
    expect(at(HOLD - 0.01).named).toBe(true);
    const after = at(HOLD);
    expect(after.named).toBe(false);
    expect(after.left.shown && after.right.shown).toBe(true);
  });

  it("ignores a pointer that arrives during it", () => {
    expect(scene(at(1.3, { hoverAt: 1 }))).toEqual(scene(at(1.3)));
  });
});

describe("after the launch", () => {
  it("brings the name back for the pointer, and is pleased to see it for a moment", () => {
    const s = at(HOLD + 1, { hoverAt: HOLD + 0.5 });
    expect(s.named).toBe(true);
    expect(s.left.gaze).toBe("pointer");
    expect(s.right.gaze).toBe("pointer");
    expect(s.left.mood).toBe("pleased");
    expect(at(HOLD + 3, { hoverAt: HOLD + 0.5 }).left.mood).toBe("idle");
  });

  it("has them find each other once a cycle, and trade something every third time", () => {
    const quiet = SETTLED + 4.5;
    expect(at(quiet).left.gaze).not.toBeNull();
    expect(at(quiet).left.gesture).toBeNull();
    expect(at(SETTLED + 6).left.gaze).toBeNull();
    const trading = SETTLED + 2 * 11;
    expect(at(trading + 4.4).left.gesture).toBe("send");
    expect(at(trading + 4.4).right.gesture).toBeNull();
    expect(at(trading + 4.6).right.gesture).toBe("receive");
  });
});

describe("next", () => {
  // The component renders at `next` and at nothing in between, so a change it
  // does not predict is a scene that stays stale until something else happens.
  it("predicts every change", () => {
    const situations: Partial<Situation>[] = [
      {},
      { waiting: true },
      { hoverAt: 9 },
      { hoverAt: 30, waiting: true },
    ];
    for (const s of situations) {
      for (let t = 0; t < 60; t += 0.07) {
        const now = at(t, s);
        const until = Math.min(now.next, 70);
        for (const u of [0.25, 0.5, 0.75, 0.999]) {
          const later = t + (until - t) * u;
          // Arriving is an event the component renders for, not a beat it waits on.
          const arrives = s.hoverAt;
          if (typeof arrives === "number" && t < arrives && later >= arrives) continue;
          expect(scene(at(later, s)), `from ${t.toFixed(2)} to ${later.toFixed(2)}`).toEqual(
            scene(now),
          );
        }
      }
    }
  });

  it("is always later than now", () => {
    for (let t = 0; t < 60; t += 0.13) expect(at(t).next).toBeGreaterThan(t);
  });
});

describe("toward", () => {
  const face = { x: 0, y: 0, width: 24, height: 24 };

  it("meets a pointer on the face straight on", () => {
    expect(toward(face, 12, 12)).toEqual([0, 0]);
  });

  it("looks the way the pointer is, and harder the further off it is, up to a hand away", () => {
    const near = toward(face, 20, 12);
    const far = toward(face, 200, 12);
    expect(near[0]).toBeGreaterThan(0);
    expect(far[0]).toBeGreaterThan(near[0]);
    expect(toward(face, 2000, 12)).toEqual(far);
    expect(toward(face, -200, 12)[0]).toBeLessThan(0);
  });

  it("goes no further up or down than an aimed look does", () => {
    expect(toward(face, 12, -500)[1]).toBe(-0.25);
    expect(toward(face, 12, 500)[1]).toBe(0.3);
  });
});
