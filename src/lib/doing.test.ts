import { describe, expect, it } from "vitest";

import { type DoingInputs, doing } from "./doing";
import type { Escalation, Outcome } from "./types";

/**
 * The words beside a name, in the rail and in the menu bar panel. The rail's
 * suite draws them in place; this holds the order they outrank each other in,
 * which both surfaces depend on.
 */

const NOW = 10 * 24 * 3_600_000;
const quiet: DoingInputs = { activity: {}, stuck: [], trail: {}, lastActive: {}, finished: {} };

function news(over: Partial<Outcome> = {}): Outcome {
  return {
    runId: "run-1",
    agentId: "a",
    groupId: "g",
    trigger: "routine",
    triggerRef: null,
    status: "done",
    headline: "Sent the digest to 12 people.",
    noteId: null,
    needs: [],
    steps: 1,
    startedAt: NOW - 6 * 3_600_000,
    finishedAt: NOW - 5 * 3_600_000,
    seenAt: null,
    ...over,
  };
}

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

  it("says what it finished and when, with the headline on hover, over the bare time", () => {
    const said = doing("a", { ...quiet, lastActive: { a: NOW }, finished: { a: news() } }, NOW);
    expect(said).toEqual({
      text: "done 5h",
      kind: "finished",
      title: "Sent the digest to 12 people.",
    });
    const failed = doing("a", { ...quiet, finished: { a: news({ status: "failed" }) } }, NOW);
    expect(failed.text).toBe("failed 5h");
  });

  it("keeps needs you, stuck and typing above finished work", () => {
    const finished = { a: news() };
    expect(
      doing("a", { ...quiet, finished, activity: { a: { state: "awaitingApproval" } } }, NOW),
    ).toEqual({ text: "needs you", kind: "asking" });
    expect(doing("a", { ...quiet, finished, stuck: [stuckOn("a", 2)] }, NOW).text).toBe("stuck 2d");
    expect(doing("a", { ...quiet, finished, activity: { a: { state: "thinking" } } }, NOW)).toEqual(
      { text: "typing", kind: "thinking" },
    );
  });

  it("falls back to when it last spoke, and to nothing for one that never has", () => {
    expect(doing("a", { ...quiet, lastActive: { a: NOW - 5 * 60_000 } }, NOW).text).toBe("5m");
    expect(doing("a", quiet, NOW)).toEqual({ text: "", kind: undefined });
  });
});
