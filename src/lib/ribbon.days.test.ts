import { describe, expect, it } from "vitest";

import { anAgent } from "../test-fixtures";
import { layoutRibbon, type RibbonInput } from "./ribbon";

/** A local moment, so the weekday is the one named whatever zone runs this. */
const at = (day: number, hour: number) => new Date(2026, 10, day, hour).getTime();

function strip(now: number): RibbonInput {
  return {
    now,
    agents: [anAgent("dara")],
    activity: {},
    outcomes: [],
    approvals: [],
    upcoming: [],
    occasions: [],
    faceSpan: 0.05,
  };
}

const dayColumns = (now: number) =>
  layoutRibbon(strip(now))
    .ticks.filter((tick) => tick.kind === "day")
    .map((tick) => new Date(tick.at).getDay());

describe("the coming days", () => {
  it("is Sunday alone on a Saturday", () => {
    expect(new Date(at(21, 12)).getDay()).toBe(6);
    expect(dayColumns(at(21, 12))).toEqual([0]);
  });

  it("is the four days after it on a Sunday", () => {
    expect(new Date(at(22, 12)).getDay()).toBe(0);
    expect(dayColumns(at(22, 12))).toEqual([1, 2, 3, 4]);
  });

  it("runs through Sunday from a Thursday", () => {
    expect(dayColumns(at(19, 12))).toEqual([5, 6, 0]);
  });
});
