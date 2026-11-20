import { describe, expect, it } from "vitest";

import { askers, questions, results, whoIsWaiting } from "./questions";

describe("the two nouns For you uses", () => {
  it("counts questions and results, never items or turns", () => {
    expect(questions(1)).toBe("1 question");
    expect(questions(3)).toBe("3 questions");
    expect(results(1)).toBe("1 result");
    expect(results(0)).toBe("0 results");
  });

  it("names who is waiting, once each, and folds the rest into a count", () => {
    expect(whoIsWaiting([])).toBe("Nobody is waiting on you");
    expect(whoIsWaiting(["Executive Assistant"])).toBe("Executive Assistant is waiting on you");
    expect(whoIsWaiting(["Pip", "A deleted agent"])).toBe(
      "Pip and a deleted agent are waiting on you",
    );
    expect(whoIsWaiting(["Pip", "A deleted agent", "Ada", "A deleted agent"])).toBe(
      "Pip, 2 deleted agents and Ada are waiting on you",
    );
    expect(whoIsWaiting(["A deleted agent", "A deleted agent"])).toBe(
      "2 deleted agents are waiting on you",
    );
    expect(whoIsWaiting(["A deleted agent", "Pip"])).toBe(
      "A deleted agent and Pip are waiting on you",
    );
    expect(whoIsWaiting(["Pip", "Ada"])).toBe("Pip and Ada are waiting on you");
    expect(whoIsWaiting(["Pip", "Ada", "Lee"])).toBe("Pip, Ada and Lee are waiting on you");
    expect(whoIsWaiting(["Pip", "Ada", "Lee", "Kai"])).toBe(
      "Pip, Ada and 2 others are waiting on you",
    );
    expect(whoIsWaiting(["Pip", "Ada", "Lee", "Kai", "Bo"])).toBe(
      "Pip, Ada and 3 others are waiting on you",
    );
  });

  it("names each agent once however many questions it asked, and two agents of one name twice", () => {
    const names: Record<string, string> = { a: "Pip", b: "Pip", c: "Ada" };
    const nameOf = (id: string) => names[id] ?? "A deleted agent";
    expect(askers([{ agentId: "a" }, { agentId: "a" }, { agentId: "c" }], nameOf)).toEqual([
      "Pip",
      "Ada",
    ]);
    expect(askers([{ agentId: "a" }, { agentId: "b" }], nameOf)).toEqual(["Pip", "Pip"]);
  });
});
