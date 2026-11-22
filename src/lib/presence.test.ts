import { describe, expect, it } from "vitest";

import { presenceLabel, presenceNote, presenceOf, QUIET } from "./presence";
import type { Activity, AgentCard, AgentId, Escalation } from "./types";

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
    runsErrands: false,
    readsWeb: true,
    browserConsent: "open",
    createdAt: 0,
    ...over,
  } as AgentCard;
}

const crew = [anAgent("a"), anAgent("b"), anAgent("c")];

function states(map: Record<string, Activity>): Record<AgentId, Activity> {
  return map as Record<AgentId, Activity>;
}

function raisedBy(by: string): Escalation {
  return {
    id: `e-${by}`,
    agentId: by as AgentId,
    groupId: "g1",
    runId: "r1",
    summary: "the deploy needs a key only you have",
    raisedAt: 0,
    saidAt: 0,
    times: 1,
    clearedAt: null,
  } as Escalation;
}

describe("presenceOf", () => {
  it("is quiet when nobody is doing anything", () => {
    expect(presenceOf(crew, states({}), [])).toEqual(QUIET);
  });

  it("counts every parked turn, not just that there is one", () => {
    const presence = presenceOf(
      crew,
      states({ a: { state: "awaitingApproval" }, c: { state: "awaitingApproval" } }),
      [],
    );
    expect(presence.blocked).toBe(2);
  });

  it("reports working without a number", () => {
    const presence = presenceOf(crew, states({ a: { state: "thinking" } }), []);
    expect(presence).toEqual({ blocked: 0, working: true });
  });

  it("counts queued work as working, because the turn is coming", () => {
    expect(presenceOf(crew, states({ b: { state: "queued", depth: 2 } }), []).working).toBe(true);
  });

  it("does not call a paused agent working", () => {
    expect(presenceOf(crew, states({ a: { state: "paused" } }), [])).toEqual(QUIET);
  });

  it("does not call an idle agent working", () => {
    expect(presenceOf(crew, states({ a: { state: "idle" } }), [])).toEqual(QUIET);
  });

  it("does not double count a parked agent as working too", () => {
    const presence = presenceOf(crew, states({ a: { state: "awaitingApproval" } }), []);
    expect(presence).toEqual({ blocked: 1, working: false });
  });

  it("reports both when one agent is parked and another is working", () => {
    const presence = presenceOf(
      crew,
      states({ a: { state: "awaitingApproval" }, b: { state: "thinking" } }),
      [],
    );
    expect(presence).toEqual({ blocked: 1, working: true });
  });

  it("ignores an agent that is not in the crew", () => {
    const presence = presenceOf([anAgent("a")], states({ z: { state: "awaitingApproval" } }), []);
    expect(presence).toEqual(QUIET);
  });

  it("counts an open escalation, which no activity state says anything about", () => {
    const presence = presenceOf(crew, states({}), [raisedBy("a")]);
    expect(presence).toEqual({ blocked: 1, working: false });
  });

  it("counts an escalation held by an agent that is working", () => {
    const presence = presenceOf(crew, states({ a: { state: "thinking" } }), [raisedBy("a")]);
    expect(presence).toEqual({ blocked: 1, working: true });
  });

  it("counts a parked turn and that agent's older escalation separately", () => {
    const presence = presenceOf(crew, states({ a: { state: "awaitingApproval" } }), [
      raisedBy("a"),
    ]);
    expect(presence.blocked).toBe(2);
  });

  it("ignores an escalation raised by an agent in another crew", () => {
    const presence = presenceOf([anAgent("a")], states({}), [raisedBy("z")]);
    expect(presence).toEqual(QUIET);
  });
});

describe("presenceNote", () => {
  it.each([
    [3, QUIET, "3 agents"],
    [1, QUIET, "1 agent"],
    [2, { blocked: 0, working: true }, "2 agents, working"],
    [2, { blocked: 1, working: false }, "2 agents, 1 question waiting on you"],
    [4, { blocked: 3, working: false }, "4 agents, 3 questions waiting on you"],
  ])("says a crew of %i as words", (count, presence, said) => {
    expect(presenceNote(count, presence)).toBe(said);
  });

  it("says waiting rather than working when both are true", () => {
    expect(presenceNote(2, { blocked: 1, working: true })).toBe(
      "2 agents, 1 question waiting on you",
    );
  });

  it("is what the label says under the name", () => {
    const presence = { blocked: 2, working: true };
    expect(presenceLabel("Sales", 5, presence)).toBe(`Sales, ${presenceNote(5, presence)}`);
  });
});

describe("the whole workspace", () => {
  it("is the sum of its crews", () => {
    const everyone = [anAgent("a"), anAgent("b", { groupId: "g2" as AgentCard["groupId"] })];
    const activity = states({ a: { state: "awaitingApproval" }, b: { state: "awaitingApproval" } });
    const parts =
      presenceOf([everyone[0]!], activity, []).blocked +
      presenceOf([everyone[1]!], activity, []).blocked;
    expect(presenceOf(everyone, activity, []).blocked).toBe(parts);
  });
});

describe("presenceLabel", () => {
  it("names the crew and its size when nothing is happening", () => {
    expect(presenceLabel("Sales", 3, QUIET)).toBe("Sales, 3 agents");
  });

  it("does not say agents about one agent", () => {
    expect(presenceLabel("Sales", 1, QUIET)).toBe("Sales, 1 agent");
  });

  it("says how many turns are waiting", () => {
    expect(presenceLabel("Sales", 3, { blocked: 2, working: false })).toBe(
      "Sales, 3 agents, 2 questions waiting on you",
    );
  });

  it("does not say turns about one turn", () => {
    expect(presenceLabel("Sales", 3, { blocked: 1, working: false })).toBe(
      "Sales, 3 agents, 1 question waiting on you",
    );
  });

  it("says working when nothing is waiting", () => {
    expect(presenceLabel("Sales", 3, { blocked: 0, working: true })).toBe(
      "Sales, 3 agents, working",
    );
  });
});
