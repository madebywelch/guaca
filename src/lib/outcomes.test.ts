import { describe, expect, it } from "vitest";

import { aGroup } from "../test-fixtures";
import {
  drawnThrough,
  EARLIER_MS,
  earlier,
  finished,
  forYouSummary,
  moreLine,
  newestByAgent,
  railMeta,
  shelves,
  statusWord,
} from "./outcomes";
import type { Outcome } from "./types";

const NOW = 30 * 24 * 3_600_000;
const HOUR = 3_600_000;

function outcome(runId: string, over: Partial<Outcome> = {}): Outcome {
  return {
    runId,
    agentId: "ada",
    groupId: "g1",
    trigger: "routine",
    triggerRef: null,
    status: "done",
    headline: `Headline of ${runId}`,
    noteId: null,
    colleagues: [],
    needs: [],
    steps: 1,
    startedAt: NOW - 2 * HOUR,
    finishedAt: NOW - HOUR,
    seenAt: null,
    ...over,
  };
}

describe("what Finished lists", () => {
  it("lists unseen runs newest first", () => {
    const listed = finished([
      outcome("old", { finishedAt: NOW - 5 * HOUR }),
      outcome("seen", { seenAt: NOW }),
      outcome("new", { finishedAt: NOW - 1 * HOUR }),
      outcome("middle", { finishedAt: NOW - 3 * HOUR }),
    ]);
    expect(listed.map((one) => one.runId)).toEqual(["new", "middle", "old"]);
  });

  it("says nothing about a check that found nothing", () => {
    expect(finished([outcome("quiet", { status: "nothingNew" })])).toEqual([]);
    expect(newestByAgent([outcome("quiet", { status: "nothingNew" })])).toEqual({});
  });

  it("orders two runs that finished in the same millisecond the same way every time", () => {
    const a = outcome("a");
    const b = outcome("b");
    expect(finished([a, b]).map((one) => one.runId)).toEqual(
      finished([b, a]).map((one) => one.runId),
    );
  });
});

describe("what Earlier lists", () => {
  it("keeps seen runs from the last seven days and nothing older or unseen", () => {
    const listed = earlier(
      [
        outcome("unseen"),
        outcome("week", { seenAt: NOW, finishedAt: NOW - EARLIER_MS }),
        outcome("older", { seenAt: NOW, finishedAt: NOW - EARLIER_MS - 1 }),
        outcome("yesterday", { seenAt: NOW, finishedAt: NOW - 24 * HOUR }),
        outcome("quiet", { seenAt: NOW, status: "nothingNew" }),
      ],
      NOW,
    );
    expect(listed.map((one) => one.runId)).toEqual(["yesterday", "week"]);
  });
});

describe("the words", () => {
  it("names an ending only when it is not a plain finish", () => {
    expect(statusWord("done")).toBeNull();
    expect(statusWord("failed")).toBe("failed");
    expect(statusWord("interrupted")).toBe("interrupted");
    expect(statusWord("stopped")).toBe("stopped");
    expect(statusWord("needsYou")).toBe("needs you");
  });

  it("puts the ending and the age on a rail row, and never says needs you there", () => {
    expect(railMeta(outcome("a", { finishedAt: NOW - 5 * HOUR }), NOW)).toBe("done 5h");
    expect(railMeta(outcome("a", { status: "failed", finishedAt: NOW - 5 * HOUR }), NOW)).toBe(
      "failed 5h",
    );
    expect(railMeta(outcome("a", { status: "needsYou", finishedAt: NOW - 5 * HOUR }), NOW)).toBe(
      "done 5h",
    );
  });

  it("leads For you with what needs the operator, and names finished work only alone", () => {
    expect(forYouSummary([], 0, 1)).toBe(
      "Nothing needs your answer. 1 did not finish since you last looked.",
    );
    expect(forYouSummary([], 0)).toBe("Nothing needs your answer right now.");
    expect(forYouSummary(["Ada", "Pip"], 4)).toBe("Ada and Pip are waiting on you.");
    expect(forYouSummary(["Executive Assistant"], 0)).toBe(
      "Executive Assistant is waiting on you.",
    );
    expect(forYouSummary([], 4)).toBe(
      "Nothing needs your answer. 4 finished since you last looked.",
    );
  });

  it("points past the panel's five lines at For you", () => {
    expect(moreLine(5, 5)).toBeNull();
    expect(moreLine(8, 5)).toBe("And 3 more in For you");
  });
});

describe("the rail's news", () => {
  it("speaks for each agent's newest unseen run", () => {
    const newest = newestByAgent([
      outcome("ada-old", { finishedAt: NOW - 4 * HOUR }),
      outcome("ada-new", { finishedAt: NOW - 2 * HOUR }),
      outcome("ada-seen", { finishedAt: NOW, seenAt: NOW }),
      outcome("bo", { agentId: "bo" }),
    ]);
    expect(newest.ada?.runId).toBe("ada-new");
    expect(newest.bo?.runId).toBe("bo");
  });
});

describe("headings", () => {
  const groups = [aGroup({ id: "g1", name: "Research" }), aGroup({ id: "g2", name: "Ops" })];

  it("draws no heading when every line is in one group", () => {
    const drawn = shelves([outcome("a"), outcome("b")], groups);
    expect(drawn).toHaveLength(1);
    expect(drawn[0]?.group).toBeNull();
  });

  it("draws one heading per group, the group with the newest line first", () => {
    const drawn = shelves(
      [
        outcome("research-old", { finishedAt: NOW - 6 * HOUR }),
        outcome("ops", { groupId: "g2", finishedAt: NOW - 3 * HOUR }),
        outcome("research-new", { finishedAt: NOW - 1 * HOUR }),
      ],
      groups,
    );
    expect(drawn.map((shelf) => shelf.group?.name)).toEqual(["Research", "Ops"]);
    expect(drawn[0]?.outcomes.map((one) => one.runId)).toEqual(["research-new", "research-old"]);
  });

  it("names a group that has gone rather than dropping its lines", () => {
    const drawn = shelves(
      [outcome("a"), outcome("b", { groupId: "gone", finishedAt: NOW - 2 * HOUR })],
      groups,
    );
    expect(drawn.map((shelf) => shelf.group?.name)).toEqual(["Research", "A deleted group"]);
  });

  it("draws nothing for nothing", () => {
    expect(shelves([], groups)).toEqual([]);
  });
});

describe("Mark all seen", () => {
  it("is bounded by the newest line drawn", () => {
    expect(drawnThrough([])).toBeNull();
    expect(drawnThrough([outcome("a", { finishedAt: 5 }), outcome("b", { finishedAt: 9 })])).toBe(
      9,
    );
  });
});
