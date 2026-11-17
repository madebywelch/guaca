import { describe, expect, it } from "vitest";

import { gaitOf } from "./clock";
import { MOODS } from "./moods";

const CREW = [
  "01JD2N4Q7K8V9X0Y1Z2A3B4C5D",
  "01JD2N4Q7K8V9X0Y1Z2A3B4C5E",
  "01JD2N4Q7K8V9X0Y1Z2A3B4C5F",
  "agent-1",
  "agent-2",
  "agent-3",
  "researcher",
  "reviewer",
  "scribe",
  "scout",
  "sentry",
  "steward",
];

function creatureTime(seed: string, wall: number): number {
  const gait = gaitOf(seed);
  return wall * gait.rate + gait.phase;
}

describe("a creature's own clock", () => {
  it("is the same one every reload", () => {
    for (const seed of CREW) expect(gaitOf(seed)).toEqual(gaitOf(seed));
  });

  it("gives a crew a spread of tempos rather than one", () => {
    const rates = CREW.map((seed) => gaitOf(seed).rate);
    expect(new Set(rates).size).toBe(CREW.length);
    expect(Math.max(...rates) - Math.min(...rates)).toBeGreaterThan(0.2);
  });

  it("does not let two creatures hold the gap they started with", () => {
    const breath = MOODS.thinking.shape.knead;
    if (!breath) throw new Error("thinking stopped breathing");
    const cycle = 1 / breath.hz;
    const drifts: number[] = [];
    for (let i = 0; i < CREW.length; i++) {
      for (let j = i + 1; j < CREW.length; j++) {
        const a = CREW[i] as string;
        const b = CREW[j] as string;
        const gap = (wall: number) => creatureTime(a, wall) - creatureTime(b, wall);
        const drift = Math.abs(gap(60) - gap(0));
        expect(drift, `${a} and ${b}`).toBeGreaterThan(0);
        drifts.push(drift);
      }
    }
    drifts.sort((a, b) => a - b);
    expect(drifts[Math.floor(drifts.length / 2)]).toBeGreaterThan(cycle / 4);
  });

  it("puts ids that differ in one character on unrelated tempos", () => {
    const near = CREW.filter((seed) => seed.startsWith("01JD"));
    const rates = near.map((seed) => gaitOf(seed).rate);
    for (let i = 0; i < rates.length; i++) {
      for (let j = i + 1; j < rates.length; j++) {
        expect(Math.abs((rates[i] as number) - (rates[j] as number))).toBeGreaterThan(0.02);
      }
    }
  });

  it("keeps every tempo close enough to the mood's own", () => {
    for (const seed of CREW) {
      const { rate, phase } = gaitOf(seed);
      expect(rate).toBeGreaterThan(0.7);
      expect(rate).toBeLessThan(1.3);
      expect(phase).toBeGreaterThanOrEqual(0);
      expect(phase).toBeLessThan(9);
    }
  });
});
