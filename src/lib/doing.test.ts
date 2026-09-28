import { describe, expect, it } from "vitest";

import { type DoingInputs, doing } from "./doing";
import type { Escalation } from "./types";

/**
 * The words beside a name, in the rail and in the menu bar panel. The rail's
 * suite draws them in place; this holds the order they outrank each other in,
 * which both surfaces depend on.
 */

const NOW = 10 * 24 * 3_600_000;
const quiet: DoingInputs = { activity: {}, stuck: [], building: {}, trail: {}, lastActive: {} };

function stuckOn(agentId: string, days: number): Escalation {
  return {
    id: "esc-1",
    agentId,
    groupId: "g",
    runId: "run-1",
    summary: "no key",
    raisedAt: NOW - days * 24 * 3_600_000,
    saidAt: NOW,
    times: 1,
    clearedAt: null,
  };
}

describe("what a row says an agent is doing", () => {
  it("says stuck over typing, because the one a person has to fix outranks the rest", () => {
    const said = doing(
      "a",
      { ...quiet, activity: { a: { state: "thinking" } }, stuck: [stuckOn("a", 2)] },
      NOW,
    );
    expect(said).toEqual({ text: "stuck 2d", kind: "asking" });
  });

  it("says a parked turn needs you, even over an older escalation", () => {
    const said = doing(
      "a",
      { ...quiet, activity: { a: { state: "awaitingApproval" } }, stuck: [stuckOn("a", 2)] },
      NOW,
    );
    expect(said).toEqual({ text: "needs you", kind: "asking" });
  });

  it("says a coding job is running after the turn that started it has ended", () => {
    expect(doing("a", { ...quiet, building: { a: "repo-1" } }, NOW)).toEqual({
      text: "writing code",
      kind: "thinking",
    });
  });

  it("falls back to when it last spoke, and to nothing for one that never has", () => {
    expect(doing("a", { ...quiet, lastActive: { a: NOW - 5 * 60_000 } }, NOW).text).toBe("5m");
    expect(doing("a", quiet, NOW)).toEqual({ text: "", kind: undefined });
  });
});
