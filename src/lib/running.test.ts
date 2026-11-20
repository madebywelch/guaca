import { describe, expect, it } from "vitest";

import { colleaguesOn, runningNote, together } from "./running";
import type { AgentId, RunId } from "./types";

const run = { runId: "run" as RunId, agentId: "owner" as AgentId };
const names: Record<string, string> = { owner: "Assistant", a: "Analyst", r: "Researcher" };

describe("together", () => {
  it("joins names the way a sentence does", () => {
    expect(together([])).toBe("");
    expect(together(["Analyst"])).toBe("Analyst");
    expect(together(["Analyst", "Researcher"])).toBe("Analyst and Researcher");
    expect(together(["Analyst", "Chef", "Researcher"])).toBe("Analyst, Chef and Researcher");
  });
});

describe("colleaguesOn", () => {
  it("names who else is working inside the conversation, and nobody on another", () => {
    const activeRun = {
      owner: "run",
      r: "run",
      a: "run",
      x: "other",
      y: undefined,
    } as unknown as Record<AgentId, RunId | undefined>;
    expect(colleaguesOn(run, activeRun, (id) => names[id])).toEqual(["Analyst", "Researcher"]);
  });

  it("leaves out a colleague that was deleted mid-turn", () => {
    const activeRun = { gone: "run" } as unknown as Record<AgentId, RunId | undefined>;
    expect(colleaguesOn(run, activeRun, (id) => names[id])).toEqual([]);
  });
});

describe("runningNote", () => {
  it("says the agent's own latest note before anything inferred", () => {
    expect(runningNote({ note: "Reading the filings" }, { state: "thinking" }, ["Analyst"])).toBe(
      "Reading the filings",
    );
  });

  it("says who it is waiting for while its colleagues work", () => {
    expect(runningNote({ note: null }, { state: "idle" }, ["Analyst", "Researcher"])).toBe(
      "Waiting for Analyst and Researcher",
    );
  });

  it("puts the person's own answer ahead of a colleague's", () => {
    expect(runningNote({ note: null }, { state: "awaitingApproval" }, ["Analyst"])).toBe(
      "Waiting for your answer",
    );
  });

  it("falls back to what the host says the agent is doing", () => {
    expect(runningNote({ note: null }, { state: "paused" }, [])).toBe("Paused");
    expect(runningNote({ note: null }, { state: "queued", depth: 2 }, [])).toBe("Waiting its turn");
    expect(runningNote({ note: null }, { state: "thinking" }, [])).toBe("Working");
    expect(runningNote({ note: null }, undefined, [])).toBe("Working");
    expect(runningNote({ note: "Drafting" }, { state: "thinking" }, ["Analyst"], true)).toBe(
      "Sending your answer",
    );
  });
});
