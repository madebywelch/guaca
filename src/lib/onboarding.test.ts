import { beforeEach, describe, expect, it, vi } from "vitest";
import { HIREABLE } from "./cafeteria";
import { FIRST_TASKS, readFirstTask, saveFirstTask } from "./onboarding";

const { origin } = vi.hoisted(() => ({ origin: vi.fn(() => "https://first.example") }));
vi.mock("./transport", () => ({ workspaceOrigin: origin }));

beforeEach(() => {
  localStorage.clear();
  origin.mockReturnValue("https://first.example");
});

describe("the first task", () => {
  it("recovers from unreadable and unknown saved drafts", () => {
    for (const value of [
      "{",
      "null",
      '{"task":"missing","brief":"Hello"}',
      '{"task":"plan","brief":7}',
    ]) {
      localStorage.setItem("guaca.firstTask.https://first.example", value);
      expect(readFirstTask()).toEqual({ task: "own", brief: "", agentId: null, sent: false });
    }
  });

  it("keeps the brief and partial completion through reloads, scoped to the host", () => {
    const draft = { task: "plan" as const, brief: "Plan a launch", agentId: "agent-1", sent: true };
    saveFirstTask(draft);
    expect(readFirstTask()).toEqual(draft);
    origin.mockReturnValue("https://second.example");
    expect(readFirstTask().brief).toBe("");
    origin.mockReturnValue("https://first.example");
    saveFirstTask(null);
    expect(readFirstTask().agentId).toBeNull();
  });

  it("offers one existing preset per task, without inventing a new agent prompt", () => {
    for (const task of FIRST_TASKS) {
      expect(HIREABLE.filter((agent) => agent.id === task.agent)).toHaveLength(1);
    }
  });
});
