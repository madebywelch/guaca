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
    const draft = {
      task: "plan" as const,
      brief: "Plan a launch",
      agentId: "agent-1",
      sent: true,
      key: "task-key",
    };
    saveFirstTask(draft);
    expect(readFirstTask()).toEqual(draft);
    origin.mockReturnValue("https://second.example");
    expect(readFirstTask().brief).toBe("");
    origin.mockReturnValue("https://first.example");
    saveFirstTask(null);
    expect(readFirstTask().agentId).toBeNull();
  });

  it("reads checkpoints written before task keys existed", () => {
    const draft = { task: "plan" as const, brief: "Saved brief", agentId: "agent-1", sent: false };
    saveFirstTask(draft);
    expect(readFirstTask()).toEqual(draft);
  });

  it("hires three different existing presets per task, led by the same coordinator", () => {
    for (const task of FIRST_TASKS) {
      const ids = task.group.map((member) => member.agent);
      expect(new Set(ids).size, task.id).toBe(3);
      expect(ids[0], task.id).toBe("chief-of-staff");
      for (const id of ids)
        expect(
          HIREABLE.filter((agent) => agent.id === id),
          id,
        ).toHaveLength(1);
    }
  });

  it("writes each suggested brief so the coordinator asks both colleagues before it asks you", () => {
    for (const task of FIRST_TASKS.filter((entry) => entry.brief)) {
      const [, first, second] = task.group.map(
        (member) => HIREABLE.find((agent) => agent.id === member.agent)!.name,
      );
      expect(task.brief, task.id).toContain(`Ask ${first} and ${second} at the same time`);
      expect(task.brief.match(/\bask me\b/gi) ?? [], task.id).toHaveLength(1);
      expect(task.brief, task.id).toMatch(
        /Ask me a question only if you cannot go on without one\.$/,
      );
    }
  });

  it("writes no em dash in anything onboarding shows", () => {
    for (const task of FIRST_TASKS) expect(JSON.stringify(task), task.id).not.toContain("\u2014");
  });
});
