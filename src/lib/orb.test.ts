import { describe, expect, it } from "vitest";

import { FORM } from "../avatars/catalog";
import { cluster, SEATS } from "./orb";

/** The viewBox every character is drawn in. `AgentAvatar` sets it. */
const BOX = 64;

/**
 * How far a face's ink reaches from its own center, as a fraction of its box.
 *
 * Taken from the catalog's construction spec rather than assumed, because the
 * seating is sized against the drawing and not against the box around it: a
 * character that grew inside `FORM` would make every ring here too wide, and
 * nothing else in the app would notice.
 */
const INK = {
  x: Math.max(FORM.right - BOX / 2, BOX / 2 - FORM.left) / BOX,
  y: Math.max(FORM.bottom - BOX / 2, BOX / 2 - FORM.top) / BOX,
};

/** The rim of the ring, which is 1px of inset shadow on a 2.4rem circle. */
const RIM = 0.5 - 1 / 38.4;

function crew(n: number): { id: string }[] {
  return Array.from({ length: n }, (_, i) => ({ id: `agent-${i}` }));
}

describe("seating a crew", () => {
  it("gives every member a seat until the ring is full, then counts the rest", () => {
    expect(cluster([]).seats).toHaveLength(0);
    expect(cluster(crew(1)).seats).toHaveLength(1);
    expect(cluster(crew(SEATS)).seats).toHaveLength(SEATS);
    expect(cluster(crew(SEATS)).rest).toBe(0);

    const crowd = cluster(crew(SEATS + 3));
    expect(crowd.seats).toHaveLength(SEATS);
    expect(crowd.rest).toBe(3);
  });

  // The point of the change: a strip of crews used to be one badge repeated,
  // because four faces in a square is what every group of two or more drew.
  it("arranges each size differently", () => {
    const shapes = new Set<string>();
    for (let n = 1; n <= SEATS; n++) {
      shapes.add(JSON.stringify(cluster(crew(n)).seats.map((s) => [s.x, s.y, s.size])));
    }
    expect(shapes.size).toBe(SEATS);
  });

  it.each(Array.from({ length: SEATS }, (_, i) => i + 1))("keeps %i inside the ring", (n) => {
    for (const seat of cluster(crew(n)).seats) {
      expect(Math.abs(seat.x - 0.5) + INK.x * seat.size).toBeLessThanOrEqual(RIM);
      expect(Math.abs(seat.y - 0.5) + INK.y * seat.size).toBeLessThanOrEqual(RIM);
    }
  });

  // Faces are the whole content of the circle. Packing one more in is not worth
  // anything if it stands where the face beside it is drawn.
  it.each(Array.from({ length: SEATS }, (_, i) => i + 1))("hides nobody at %i", (n) => {
    const seats = cluster(crew(n)).seats;
    for (const a of seats) {
      for (const b of seats) {
        if (a === b) continue;
        const apart = Math.hypot(a.x - b.x, a.y - b.y);
        expect(apart).toBeGreaterThanOrEqual(INK.x * 2 * a.size);
      }
    }
  });

  // The lean is decoration, and decoration that moves is a crew rearranging
  // itself for no reason every time the rail re-renders.
  it("leans a member the same way every time", () => {
    const once = cluster(crew(4)).seats.map((s) => s.tilt);
    const again = cluster(crew(4)).seats.map((s) => s.tilt);

    expect(again).toEqual(once);
    for (const tilt of once) expect(Math.abs(tilt)).toBeLessThanOrEqual(6);
  });

  // Ids are UUIDs, which any hash spreads. Short seeds are what catch one out:
  // a rolling multiply-add gave `cook 1`, `cook 2` and `cook 3` leans a tenth
  // of a degree apart, which is a crew drawn as one face repeated, and the only
  // place it would have shown is a screenshot nobody took.
  it("leans seeds that differ in one character differently", () => {
    const leans = cluster([{ id: "cook 1" }, { id: "cook 2" }, { id: "cook 3" }]).seats.map(
      (s) => s.tilt,
    );

    expect(new Set(leans).size).toBe(3);
  });

  it("stands a crew of one straight", () => {
    expect(cluster(crew(1)).seats[0]?.tilt).toBe(0);
  });
});
