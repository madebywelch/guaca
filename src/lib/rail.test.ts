import { describe, expect, it } from "vitest";

import { landsBefore, numbered, railOrder } from "./rail";
import type { Activity, AgentCard, AgentId } from "./types";

function agent(name: string, over: Partial<AgentCard> = {}): AgentCard {
  return {
    id: name,
    groupId: "g1",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    readsWeb: true,
    browserConsent: "open",
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "m",
    subscriptionModel: "",
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
    const tied = [agent("First"), agent("Second"), agent("Third")];
    expect(drawn(tied)).toEqual(["First", "Second", "Third"]);
  });
});

describe("what activity is lent", () => {
  it("lifts a working row to the top of its section and gives the place back", () => {
    const rows = crew();
    const working: Record<AgentId, Activity> = { Scribe: { state: "thinking" } };
    expect(drawn(rows, working)).toEqual(["Scribe", "Manager", "Cook"]);
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
    const rows = crew();
    expect(drawn(rows, { Scribe: { state: "paused" } })).toEqual(["Manager", "Cook", "Scribe"]);
  });

  it("never lifts a pinned row, which is the whole of what a pin is for", () => {
    const rows = [
      agent("Manager", { railOrder: 0, pinned: true }),
      agent("Cook", { railOrder: 1, pinned: true }),
      agent("Scribe", { railOrder: 2 }),
    ];
    expect(drawn(rows, { Cook: { state: "thinking" } })).toEqual(["Manager", "Cook", "Scribe"]);
  });

  it("lends nothing while a drag is in progress", () => {
    const rows = crew();
    const working: Record<AgentId, Activity> = { Scribe: { state: "thinking" } };
    expect(drawn(rows, working, {}, { frozen: true })).toEqual(["Manager", "Cook", "Scribe"]);
  });

  it("keeps a pin at the head of its crew, over the arrangement and over a turn", () => {
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
    const rows = crew();
    expect(landsBefore(rows, "Outsider", "Cook")).toBe("Cook");
  });

  it("asks for nothing when a row is dropped on itself", () => {
    const rows = crew();
    expect(landsBefore(rows, "Cook", "Cook")).toBeUndefined();
    expect(landsBefore(rows, "Cook", "Gone")).toBeUndefined();
  });

  it("reads a column of crews the way it reads a section of rows", () => {
    const column = [{ id: "Everyone" }, { id: "Research" }, { id: "Kitchen" }];
    expect(landsBefore(column, "Everyone", "Research")).toBe("Kitchen");
    expect(landsBefore(column, "Everyone", "Kitchen")).toBeNull();
    expect(landsBefore(column, "Kitchen", "Everyone")).toBe("Everyone");
  });
});

describe("the numbers Command 1 to 9 open", () => {
  it("count group by group in the groups' order, pinned first, and ignore who is working", () => {
    const rows = [
      agent("Cook", { railOrder: 1 }),
      agent("Manager", { railOrder: 0 }),
      agent("Scribe", { railOrder: 2, pinned: true }),
      agent("Clerk", { groupId: "g2" }),
      agent("Gone", { lifecycle: "terminated" }),
    ];
    expect(names(numbered(rows, [{ id: "g2" }, { id: "g1" }]))).toEqual([
      "Clerk",
      "Scribe",
      "Manager",
      "Cook",
    ]);
  });

  it("keep an agent whose group is not listed, at the end", () => {
    expect(names(numbered([agent("Lost", { groupId: "g9" }), ...crew()], [{ id: "g1" }]))).toEqual([
      "Manager",
      "Cook",
      "Scribe",
      "Lost",
    ]);
  });
});
