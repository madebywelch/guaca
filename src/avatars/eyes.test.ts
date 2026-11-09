import { describe, expect, it } from "vitest";

import { type Mass, settle } from "./eyes";

describe("gaze integration", () => {
  it("converges without escaping at the permitted 50 ms frame step", () => {
    const mass: Mass = { gaze: [0, 0], vel: [0, 0] };
    for (let frame = 0; frame < 120; frame++) {
      settle(mass, [0.3, 0], 0.05);
      expect(mass.gaze[0]).toBeGreaterThanOrEqual(0);
      expect(mass.gaze[0]).toBeLessThanOrEqual(0.3);
      expect(Number.isFinite(mass.vel[0])).toBe(true);
    }
    expect(mass.gaze[0]).toBeCloseTo(0.3, 5);
  });

  it("holds at zero elapsed time and converges for permitted steps in every direction", () => {
    for (let ms = 1; ms <= 50; ms++) {
      for (let direction = 0; direction < 16; direction++) {
        const angle = (direction * Math.PI) / 8;
        const target: [number, number] = [0.3 * Math.cos(angle), 0.3 * Math.sin(angle)];
        const mass: Mass = { gaze: [0, 0], vel: [0, 0] };
        settle(mass, target, 0);
        expect(mass).toEqual({ gaze: [0, 0], vel: [0, 0] });
        for (let frame = 0; frame < Math.ceil(2 / (ms / 1000)); frame++) {
          settle(mass, target, ms / 1000);
          expect(mass.gaze.every(Number.isFinite)).toBe(true);
          expect(mass.vel.every(Number.isFinite)).toBe(true);
          expect(Math.hypot(...mass.gaze)).toBeLessThanOrEqual(0.300000001);
        }
        expect(mass.gaze[0]).toBeCloseTo(target[0], 5);
        expect(mass.gaze[1]).toBeCloseTo(target[1], 5);
      }
    }
  });

  it("remains finite through reversals and a long stall", () => {
    const mass: Mass = { gaze: [0, 0], vel: [0, 0] };
    for (let frame = 0; frame < 100; frame++) {
      settle(mass, [frame % 2 ? -0.3 : 0.3, 0.3], 0.05);
      expect(Math.hypot(...mass.gaze)).toBeLessThan(0.5);
      expect(mass.vel.every(Number.isFinite)).toBe(true);
    }
    settle(mass, [-0.3, -0.3], 60);
    expect(mass.gaze[0]).toBeCloseTo(-0.3, 5);
    expect(mass.gaze[1]).toBeCloseTo(-0.3, 5);
    expect(mass.vel.every(Number.isFinite)).toBe(true);
  });
});
