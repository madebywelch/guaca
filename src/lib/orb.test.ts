import { describe, expect, it } from "vitest";

import { FORM } from "../avatars/form";
import { cluster, SEATS } from "./orb";

const INK = { most: FORM.reach / FORM.box, rest: FORM.radius / FORM.box };

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

  it("arranges each size differently", () => {
    const shapes = new Set<string>();
    for (let n = 1; n <= SEATS; n++) {
      shapes.add(JSON.stringify(cluster(crew(n)).seats.map((s) => [s.x, s.y, s.size])));
    }
    expect(shapes.size).toBe(SEATS);
  });

  it.each(Array.from({ length: SEATS }, (_, i) => i + 1))("keeps %i inside the ring", (n) => {
    for (const seat of cluster(crew(n)).seats) {
      expect(Math.abs(seat.x - 0.5) + INK.most * seat.size).toBeLessThanOrEqual(RIM);
      expect(Math.abs(seat.y - 0.5) + INK.most * seat.size).toBeLessThanOrEqual(RIM);
    }
  });

  it.each(Array.from({ length: SEATS }, (_, i) => i + 1))("hides nobody at %i", (n) => {
    const seats = cluster(crew(n)).seats;
    for (const a of seats) {
      for (const b of seats) {
        if (a === b) continue;
        const apart = Math.hypot(a.x - b.x, a.y - b.y);
        expect(apart).toBeGreaterThanOrEqual(INK.rest * 2 * a.size);
      }
    }
  });

  it("leans a member the same way every time", () => {
    const once = cluster(crew(4)).seats.map((s) => s.tilt);
    const again = cluster(crew(4)).seats.map((s) => s.tilt);

    expect(again).toEqual(once);
    for (const tilt of once) expect(Math.abs(tilt)).toBeLessThanOrEqual(6);
  });

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
