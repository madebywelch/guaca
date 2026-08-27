import { describe, expect, it } from "vitest";

import { landsBefore, nudgeTarget, railOrder } from "./rail";
import type { Activity, AgentCard, AgentId } from "./types";

function agent(name: string, over: Partial<AgentCard> = {}): AgentCard {
  return {
    id: name,
    groupId: "g1",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    repositoryId: null,
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
    ...over,
  };
}

/** Three agents in the order the operator arranged them. */
function crew(): AgentCard[] {
  return [
    agent("Manager", { railOrder: 0 }),
    agent("Cook", { railOrder: 1 }),
    agent("Scribe", { railOrder: 2 }),
  ];
}

function names(rows: AgentCard[]): string[] {
  return rows.map((row) => row.name);
}

function drawn(
  rows: AgentCard[],
  activity: Record<AgentId, Activity> = {},
  lastActive: Record<AgentId, number> = {},
  options: { frozen?: boolean } = {},
): string[] {
  return names(railOrder(rows, { activity, lastActive, ...options }));
}

describe("the arrangement", () => {
  it("is what the rail draws when nothing is happening", () => {
    // The old rail was ordered by who spoke last and by nothing else, so a
    // conversation rewrote it and no arrangement could survive one.
    expect(drawn(crew(), {}, { Scribe: 9_000, Cook: 5_000 })).toEqual([
      "Manager",
      "Cook",
      "Scribe",
    ]);
  });

  it("does not depend on the order it was handed", () => {
    const shuffled = [...crew()].reverse();
    expect(drawn(shuffled)).toEqual(["Manager", "Cook", "Scribe"]);
  });

  it("breaks a tie by the order the rows arrived in", () => {
    // Every row is at 0 immediately after an upgrade, and a rail that reorders
    // itself on launch is a rail that lost the arrangement it was preserving.
    const tied = [agent("First"), agent("Second"), agent("Third")];
    expect(drawn(tied)).toEqual(["First", "Second", "Third"]);
  });
});

describe("what activity is lent", () => {
  it("lifts a working row to the top of its section and gives the place back", () => {
    const rows = crew();
    const working: Record<AgentId, Activity> = { Scribe: { state: "thinking" } };
    expect(drawn(rows, working)).toEqual(["Scribe", "Manager", "Cook"]);
    // The same rail a moment later, with the turn finished.
    expect(drawn(rows, {}, { Scribe: 9_000 })).toEqual(["Manager", "Cook", "Scribe"]);
  });

  it("puts the one asking for something above the ones working", () => {
    const rows = crew();
    const state: Record<AgentId, Activity> = {
      Manager: { state: "thinking" },
      Scribe: { state: "awaitingApproval" },
      Cook: { state: "queued", depth: 2 },
    };
    expect(drawn(rows, state)).toEqual(["Scribe", "Manager", "Cook"]);
  });

  it("separates two working rows by who spoke last and nothing else", () => {
    const rows = crew();
    const state: Record<AgentId, Activity> = {
      Manager: { state: "thinking" },
      Scribe: { state: "thinking" },
    };
    expect(drawn(rows, state, { Manager: 1_000, Scribe: 9_000 })).toEqual([
      "Scribe",
      "Manager",
      "Cook",
    ]);
  });

  it("leaves a paused row where the operator put it", () => {
    // Paused is not work in progress, it is a row that will not move until
    // somebody moves it, so lifting it would hold the top indefinitely.
    const rows = crew();
    expect(drawn(rows, { Scribe: { state: "paused" } })).toEqual(["Manager", "Cook", "Scribe"]);
  });

  it("never lifts a pinned row, which is the whole of what a pin is for", () => {
    // Both pinned, so both are in the band a pin holds. The one that is working
    // still does not move: being in the same place every time is what a pin is
    // for, and a row that climbs when its agent gets busy is what it stops.
    const rows = [
      agent("Manager", { railOrder: 0, pinned: true }),
      agent("Cook", { railOrder: 1, pinned: true }),
      agent("Scribe", { railOrder: 2 }),
    ];
    expect(drawn(rows, { Cook: { state: "thinking" } })).toEqual(["Manager", "Cook", "Scribe"]);
  });

  it("lends nothing while a drag is in progress", () => {
    // Dragging is arranging, so it has to operate on the arrangement: a row
    // dropped under a peer that is only near the top because it is mid-turn
    // would land somewhere nobody aimed at.
    const rows = crew();
    const working: Record<AgentId, Activity> = { Scribe: { state: "thinking" } };
    expect(drawn(rows, working, {}, { frozen: true })).toEqual(["Manager", "Cook", "Scribe"]);
  });

  it("keeps a pin at the head of its crew, over the arrangement and over a turn", () => {
    // Every section is a crew or part of one, so there is no section a pin does
    // not head. It used to hold only where the rail drew a pinned section of its
    // own, which was the overview: going inside the crew drew the list without
    // it, and pinning a row while looking at that list moved nothing.
    const rows = [
      agent("Manager", { railOrder: 0 }),
      agent("Cook", { railOrder: 1 }),
      agent("Scribe", { railOrder: 2, pinned: true }),
    ];
    expect(drawn(rows, { Cook: { state: "thinking" } })).toEqual(["Scribe", "Cook", "Manager"]);
  });
});

describe("where a dropped row lands", () => {
  it("goes after the row it passed on the way down", () => {
    const rows = crew();
    expect(landsBefore(rows, "Manager", "Cook")).toBe("Scribe");
  });

  it("is the end of the group when it passed the last row", () => {
    const rows = crew();
    expect(landsBefore(rows, "Manager", "Scribe")).toBeNull();
  });

  it("goes in front of the row it stopped on coming up", () => {
    const rows = crew();
    expect(landsBefore(rows, "Scribe", "Manager")).toBe("Manager");
  });

  it("goes in front of the target when it came from another group", () => {
    // An agent arriving from elsewhere has no place in this section to have
    // traveled from, so there is no direction to read.
    const rows = crew();
    expect(landsBefore(rows, "Outsider", "Cook")).toBe("Cook");
  });

  it("asks for nothing when a row is dropped on itself", () => {
    // The runtime treats an anchor it cannot find as the end of the group, so a
    // null gesture that reached it would move the row to the bottom.
    const rows = crew();
    expect(landsBefore(rows, "Cook", "Cook")).toBeUndefined();
    expect(landsBefore(rows, "Cook", "Gone")).toBeUndefined();
  });
});

describe("one row at a time", () => {
  it("swaps with the row above or below", () => {
    const rows = crew();
    expect(nudgeTarget(rows, "Cook", -1)).toBe("Manager");
    expect(nudgeTarget(rows, "Cook", 1)).toBeNull();
    expect(nudgeTarget(rows, "Manager", 1)).toBe("Scribe");
  });

  it("says there is nowhere to go at either end", () => {
    const rows = crew();
    expect(nudgeTarget(rows, "Manager", -1)).toBeUndefined();
    expect(nudgeTarget(rows, "Scribe", 1)).toBeUndefined();
    expect(nudgeTarget(rows, "Nobody", -1)).toBeUndefined();
  });
});
