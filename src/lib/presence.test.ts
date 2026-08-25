import { describe, expect, it } from "vitest";

import { presenceLabel, presenceOf, QUIET } from "./presence";
import type { Activity, AgentCard, AgentId } from "./types";

function anAgent(id: string, over: Partial<AgentCard> = {}): AgentCard {
  return {
    id: id as AgentId,
    name: id,
    avatar: "blank",
    color: "#888888",
    role: "",
    model: "",
    groupId: "g1" as AgentCard["groupId"],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    hasComputer: false,
    hasBrowser: false,
    repositoryId: null,
    createdAt: 0,
    ...over,
  } as AgentCard;
}

const crew = [anAgent("a"), anAgent("b"), anAgent("c")];

function states(map: Record<string, Activity>): Record<AgentId, Activity> {
  return map as Record<AgentId, Activity>;
}

describe("presenceOf", () => {
  it("is quiet when nobody is doing anything", () => {
    expect(presenceOf(crew, states({}))).toEqual(QUIET);
  });

  it("counts every parked turn, not just that there is one", () => {
    const presence = presenceOf(
      crew,
      states({ a: { state: "awaitingApproval" }, c: { state: "awaitingApproval" } }),
    );
    expect(presence.blocked).toBe(2);
  });

  it("reports working without a number", () => {
    const presence = presenceOf(crew, states({ a: { state: "thinking" } }));
    expect(presence).toEqual({ blocked: 0, working: true });
  });

  it("counts queued work as working, because the turn is coming", () => {
    expect(presenceOf(crew, states({ b: { state: "queued", depth: 2 } })).working).toBe(true);
  });

  // A paused row is not work in progress. It scores nothing in `liftOf` for the
  // same reason, and a ring lit for one would never go out.
  it("does not call a paused agent working", () => {
    expect(presenceOf(crew, states({ a: { state: "paused" } }))).toEqual(QUIET);
  });

  it("does not call an idle agent working", () => {
    expect(presenceOf(crew, states({ a: { state: "idle" } }))).toEqual(QUIET);
  });

  // A parked agent is not also counted as working: it is the one thing the
  // count is for, and a ring around a number says the crew is busy when in fact
  // it has stopped.
  it("does not double count a parked agent as working too", () => {
    const presence = presenceOf(crew, states({ a: { state: "awaitingApproval" } }));
    expect(presence).toEqual({ blocked: 1, working: false });
  });

  it("reports both when one agent is parked and another is working", () => {
    const presence = presenceOf(
      crew,
      states({ a: { state: "awaitingApproval" }, b: { state: "thinking" } }),
    );
    expect(presence).toEqual({ blocked: 1, working: true });
  });

  // The activity map is workspace-wide and the column draws one crew at a time.
  it("ignores an agent that is not in the crew", () => {
    const presence = presenceOf([anAgent("a")], states({ z: { state: "awaitingApproval" } }));
    expect(presence).toEqual(QUIET);
  });
});

describe("the whole workspace", () => {
  // The menu bar's number is the workspace total and the column's numbers are
  // its parts. One fold produces both, so the strip and the column cannot
  // report different counts of the same parked turns.
  it("is the sum of its crews", () => {
    const everyone = [anAgent("a"), anAgent("b", { groupId: "g2" as AgentCard["groupId"] })];
    const activity = states({ a: { state: "awaitingApproval" }, b: { state: "awaitingApproval" } });
    const parts =
      presenceOf([everyone[0]!], activity).blocked + presenceOf([everyone[1]!], activity).blocked;
    expect(presenceOf(everyone, activity).blocked).toBe(parts);
  });
});

describe("presenceLabel", () => {
  it("names the crew and its size when nothing is happening", () => {
    expect(presenceLabel("Sales", 3, QUIET)).toBe("Sales, 3 agents");
  });

  it("does not say agents about one agent", () => {
    expect(presenceLabel("Sales", 1, QUIET)).toBe("Sales, 1 agent");
  });

  // The count in the corner is the whole point of the column and it is a
  // picture. Anything that cannot see it has to be told.
  it("says how many turns are waiting", () => {
    expect(presenceLabel("Sales", 3, { blocked: 2, working: false })).toBe(
      "Sales, 3 agents, 2 turns waiting on you",
    );
  });

  it("does not say turns about one turn", () => {
    expect(presenceLabel("Sales", 3, { blocked: 1, working: false })).toBe(
      "Sales, 3 agents, 1 turn waiting on you",
    );
  });

  it("says working when nothing is waiting", () => {
    expect(presenceLabel("Sales", 3, { blocked: 0, working: true })).toBe(
      "Sales, 3 agents, working",
    );
  });
});
