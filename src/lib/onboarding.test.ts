import { beforeEach, describe, expect, it, vi } from "vitest";
import { HIREABLE } from "./cafeteria";
import {
  crewFor,
  FIRST_TASKS,
  heldStart,
  holdStart,
  named,
  readCrewOffer,
  readFirstTask,
  saveCrewOffer,
  saveFirstTask,
} from "./onboarding";

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
      localStorage.setItem("guaca.firstGroup.https://first.example", value);
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

  it("ignores a checkpoint left by the single-agent first task, which a group cannot continue", () => {
    localStorage.setItem(
      "guaca.firstTask.https://first.example",
      JSON.stringify({
        task: "plan",
        brief: "Old brief",
        agentId: null,
        sent: false,
        key: "old-key",
      }),
    );
    expect(readFirstTask()).toEqual({ task: "own", brief: "", agentId: null, sent: false });
  });

  it("reads checkpoints written before task keys existed", () => {
    const draft = { task: "plan" as const, brief: "Saved brief", agentId: "agent-1", sent: false };
    saveFirstTask(draft);
    expect(readFirstTask()).toEqual(draft);
  });

  it("hires one existing preset for a suggested task, and keeps a coordinator and a colleague for later", () => {
    for (const task of FIRST_TASKS.filter((entry) => entry.brief)) {
      const crew = crewFor(task.id);
      expect(task.group, task.id).toHaveLength(1);
      expect(crew, task.id).not.toBeNull();
      const ids = [...task.group, ...crew!.group].map((member) => member.agent);
      expect(new Set(ids).size, task.id).toBe(3);
      expect(ids[0], task.id).not.toBe("chief-of-staff");
      expect(ids[1], task.id).toBe("chief-of-staff");
      for (const id of ids)
        expect(
          HIREABLE.filter((agent) => agent.id === id),
          id,
        ).toHaveLength(1);
    }
  });

  it("still hires a group of three, led by the coordinator, for a task in the person's own words", () => {
    const own = FIRST_TASKS.find((task) => task.id === "own")!;
    expect(own.group.map((member) => member.agent)).toEqual([
      "chief-of-staff",
      "market-researcher",
      "product-manager",
    ]);
    expect(crewFor("own")).toBeNull();
  });

  it("writes each suggested brief for one agent to finish alone, naming no colleague", () => {
    const names = HIREABLE.map((agent) => agent.name);
    for (const task of FIRST_TASKS.filter((entry) => entry.brief)) {
      expect(
        names.filter((name) => task.brief.includes(name)),
        task.id,
      ).toEqual([]);
      expect(task.brief, task.id).not.toMatch(/\bask\b(?! me)/i);
      expect(task.brief.match(/\bask me\b/gi) ?? [], task.id).toHaveLength(1);
      expect(task.brief, task.id).toMatch(
        /Ask me a question only if you cannot go on without one\.$/,
      );
    }
  });

  it("gives research the figures its verdict turns on, and asks for the sums and the loan still owed", () => {
    const brief = FIRST_TASKS.find((task) => task.id === "research")!.brief;
    expect(brief).toMatch(/It costs \$35,000 and should be worth about \$20,000 in three years\./);
    expect(brief).toMatch(/borrow at 6% over five years with nothing down/);
    expect(brief).toMatch(/\$500 a month with \$3,000 down/);
    expect(brief).toMatch(/Show the arithmetic/);
    expect(brief).toMatch(/loan still owed at the end of the three years/);
    expect(brief).toMatch(/pick on those numbers/);
    expect(brief).toMatch(/assume a typical one and say so/);
  });

  it("has the plan assume a typical detail and say so, so it never answers with questions alone", () => {
    const brief = FIRST_TASKS.find((task) => task.id === "plan")!.brief;
    expect(brief).toMatch(
      /Where you need a detail I have not given.*assume a typical one and say so\./,
    );
  });

  it("has the draft bracket every fact it was not given instead of inventing one", () => {
    const brief = FIRST_TASKS.find((task) => task.id === "write")!.brief;
    expect(brief).toMatch(/Say nothing I have not told you/);
    expect(brief).toMatch(/a placeholder in brackets such as \[time\]/);
    expect(brief).not.toMatch(/assume/);
  });

  it("writes each deeper pass so the coordinator asks the first agent and the new colleague at once", () => {
    for (const task of FIRST_TASKS.filter((entry) => entry.brief)) {
      const crew = crewFor(task.id)!;
      const [solo, , colleague] = [...task.group, ...crew.group].map(
        (member) => HIREABLE.find((agent) => agent.id === member.agent)!.name,
      );
      expect(crew.brief, task.id).toContain(`Ask ${solo} and ${colleague} at the same time`);
      expect(crew.brief.split("{brief}"), task.id).toHaveLength(2);
      expect(crew.brief, task.id).toMatch(
        /Ask me a question only if you cannot go on without one\.$/,
      );
      expect(crew.offer, task.id).toContain(solo);
      expect(crew.offer, task.id).toContain(colleague);
    }
  });

  it("writes the names the host gave into a text, and leaves names it kept alone", () => {
    expect(
      named(
        "Ask Product Manager and QA Tester at the same time.",
        new Map([
          ["Product Manager", "Product Manager"],
          ["QA Tester", "QA Tester 2"],
        ]),
      ),
    ).toBe("Ask Product Manager and QA Tester 2 at the same time.");
  });

  it("keeps the crew offer through reloads, scoped to the host, and forgets it once cleared", () => {
    const offer = { task: "research" as const, agentId: "agent-1", brief: "Buy or lease?" };
    saveCrewOffer(offer);
    expect(readCrewOffer()).toEqual(offer);
    saveCrewOffer({ ...offer, key: "crew-key" });
    expect(readCrewOffer()?.key).toBe("crew-key");
    origin.mockReturnValue("https://second.example");
    expect(readCrewOffer()).toBeNull();
    origin.mockReturnValue("https://first.example");
    saveCrewOffer(null);
    expect(readCrewOffer()).toBeNull();
  });

  it("offers no crew from an unreadable checkpoint or for a task that has none", () => {
    for (const value of [
      "{",
      "null",
      '{"task":"own","agentId":"agent-1","brief":"Hello"}',
      '{"task":"plan","agentId":7,"brief":"Hello"}',
      '{"task":"plan","agentId":"agent-1"}',
    ]) {
      localStorage.setItem("guaca.crewOffer.https://first.example", value);
      expect(readCrewOffer(), value).toBeNull();
    }
  });

  it("writes no em dash in anything onboarding shows", () => {
    for (const task of FIRST_TASKS) expect(JSON.stringify(task), task.id).not.toContain("\u2014");
  });
});

describe("what is held before the host exists", () => {
  it("lives in memory only, so a key entered early never reaches this device's storage", () => {
    const setItem = vi.spyOn(Storage.prototype, "setItem");
    holdStart({
      task: "plan",
      brief: "Plan a launch",
      step: "start",
      provider: {
        provider: "compatible",
        baseUrl: "https://openrouter.ai/api/v1",
        model: "a-model",
        apiKey: "sk-held",
        subscriptionModel: "",
        effort: "auto",
      },
      chosen: true,
      queued: true,
      focus: null,
    });
    expect(heldStart()?.provider?.apiKey).toBe("sk-held");
    expect(setItem).not.toHaveBeenCalled();
    expect(localStorage.length).toBe(0);
    holdStart(null);
    expect(heldStart()).toBeNull();
  });
});
