import { describe, expect, it } from "vitest";

import { MOODS } from "../../src/avatars/moods";
import { DIRECTIONS, type Flags, HOLD, type Happening } from "./scenes";

const HAPPENINGS: Record<Happening, true> = {
  none: true,
  message: true,
  working: true,
  waiting: true,
  asleep: true,
};

function* everyMoment(happening: Happening): Generator<[number, Flags]> {
  for (const hover of [false, true]) {
    for (const visits of [0, 1, 4]) {
      const flags: Flags = {
        condensedAt: HOLD,
        hover,
        hoverAt: 7.5,
        visits,
        happening,
        happeningAt: 5,
      };
      for (let t = 0; t <= 40; t += 0.05) yield [t, flags];
    }
  }
}

describe("the brand mark's scenes", () => {
  for (const direction of DIRECTIONS) {
    it(`spend the amber mark in ${direction.name} on a turn parked on the operator, and on nothing else`, () => {
      const amber = new Set<Happening>();
      for (const happening of Object.keys(HAPPENINGS) as Happening[]) {
        for (const [t, flags] of everyMoment(happening)) {
          const scene = direction.at(t, flags);
          if (scene.cues.some((cue) => MOODS[cue.mood].mark === "bang")) amber.add(happening);
        }
      }
      expect([...amber]).toEqual(["waiting"]);
    });
  }
});
