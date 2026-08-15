import { describe, expect, it } from "vitest";

import { why } from "./WireRow";

describe("why", () => {
  it("keeps what happened and drops the instructions to the model", () => {
    // Guard refusals are written for a model reading them mid-turn: a label, a
    // reason, and what to do instead. A chip wants the middle one.
    expect(
      why(
        "Refused: you already sent Chef this exact message in this run. Repeating it will not produce a different reply. Move on.",
      ),
    ).toBe("you already sent Chef this exact message in this run");
  });

  it("survives a reason that is one sentence, or none of the expected shape", () => {
    expect(why("Refused: no agent named Ghost exists")).toBe("no agent named Ghost exists");
    expect(why("something went wrong")).toBe("something went wrong");
  });
});
