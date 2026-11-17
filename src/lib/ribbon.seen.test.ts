import { describe, expect, it } from "vitest";

import { anAgent, anApproval, anOutcome } from "../test-fixtures";
import { layoutRibbon, type RibbonInput } from "./ribbon";

const at = (day: number, hour: number, minute = 0) =>
  new Date(2026, 10, day, hour, minute).getTime();

function strip(over: Partial<RibbonInput>): RibbonInput {
  return {
    now: at(16, 9, 12),
    agents: ["ines", "tomas", "noor", "dara"].map((id) => anAgent(id)),
    activity: {},
    outcomes: [],
    approvals: [],
    upcoming: [],
    occasions: [],
    faceSpan: 0.05,
    ...over,
  };
}

describe("the band after looking at one agent", () => {
  it("still holds every run nobody has seen", () => {
    const outcomes = [
      anOutcome({ agentId: "ines", startedAt: at(16, 2, 40), finishedAt: at(16, 2, 56) }),
      anOutcome({ agentId: "tomas", startedAt: at(16, 4), finishedAt: at(16, 4, 10) }),
      anOutcome({
        agentId: "noor",
        startedAt: at(16, 6, 20),
        finishedAt: at(16, 6, 30),
        seenAt: at(16, 9, 10),
      }),
    ];
    const band = layoutRibbon(strip({ outcomes })).band;
    expect(band).not.toBeNull();
    for (const run of outcomes.filter((one) => one.seenAt === null)) {
      expect(run.finishedAt).toBeGreaterThanOrEqual(band?.from ?? Number.POSITIVE_INFINITY);
    }
  });
});

describe("a request from an agent that is gone", () => {
  it("has no point on the strip", () => {
    const approvals = [
      anApproval({ agentId: "deleted", createdAt: at(16, 9, 5), lapsesAt: at(16, 9, 15) }),
    ];
    expect(layoutRibbon(strip({ approvals })).lapses).toEqual([]);
  });
});
