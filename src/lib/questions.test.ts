import { describe, expect, it } from "vitest";

import { questions, results, whoIsWaiting } from "./questions";

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
    expect(whoIsWaiting(["Pip", "Pip"])).toBe("Pip is waiting on you");
    expect(whoIsWaiting(["Pip", "Ada"])).toBe("Pip and Ada are waiting on you");
    expect(whoIsWaiting(["Pip", "Ada", "Lee"])).toBe("Pip, Ada and Lee are waiting on you");
    expect(whoIsWaiting(["Pip", "Ada", "Lee", "Kai"])).toBe(
      "Pip, Ada and 2 others are waiting on you",
    );
    expect(whoIsWaiting(["Pip", "Ada", "Lee", "Kai", "Bo"])).toBe(
      "Pip, Ada and 3 others are waiting on you",
    );
  });
});
