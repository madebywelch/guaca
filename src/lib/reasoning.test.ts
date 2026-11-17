import { describe, expect, it } from "vitest";

import { keepThought, thoughtNow } from "./reasoning";

describe("keeping a thought", () => {
  it("appends what arrives", () => {
    expect(keepThought(undefined, "I need")).toBe("I need");
    expect(keepThought("I need", " the total")).toBe("I need the total");
  });

  it("keeps a whole turn's working, not the last line of it", () => {
    let held = "";
    for (let i = 0; i < 400; i += 1) held = keepThought(held, `sentence ${i}. `);

    expect(held.startsWith("sentence 0.")).toBe(true);
    expect(held).toContain("sentence 399.");
  });

  it("keeps the end once it is longer than anything anyone will read", () => {
    const held = keepThought("a".repeat(200_000), "END");
    expect(held.endsWith("END")).toBe(true);
    expect(held.length).toBe(200_000);
  });

  it("keeps the newlines, because they are where a line starts", () => {
    const held = keepThought(keepThought("", "Checking totals.\n\n"), "Now the sum.");
    expect(thoughtNow(held).line).toBe("Now the sum.");
  });
});

describe("the line drawn from it", () => {
  it("is the last sentence that finished, not the one being written", () => {
    const half = thoughtNow("The totals are in. Now I need to check the third");
    expect(half.line).toBe("The totals are in.");

    const whole = thoughtNow("The totals are in. Now I need to check the third quarter.");
    expect(whole.line).toBe("Now I need to check the third quarter.");
  });

  it("draws the sentence being written only until the first one lands", () => {
    expect(thoughtNow("They want the total").line).toBe("They want the total");
    expect(thoughtNow("They want the total. So 17").line).toBe("They want the total.");
  });

  it("counts a line ended by a newline as finished, period or not", () => {
    expect(thoughtNow("- check the totals\n- then the ledger").line).toBe("- check the totals");
  });

  it("holds the previous sentence while the next one is still blank", () => {
    expect(thoughtNow("Checking totals.\n\n").line).toBe("Checking totals.");
  });

  it("is empty when there is nothing to say", () => {
    expect(thoughtNow(undefined)).toEqual({ heading: "", line: "" });
    expect(thoughtNow("\n\n  \n")).toEqual({ heading: "", line: "" });
  });

  it("takes the marks off rather than drawing them", () => {
    expect(thoughtNow("running `update_memory` next.").line).toBe("running update_memory next.");
  });

  it("leaves an identifier alone", () => {
    expect(thoughtNow("call update_memory with the whole file.").line).toBe(
      "call update_memory with the whole file.",
    );
  });

  it("clips a long sentence from the end, where it is read from", () => {
    const line = `${"word ".repeat(60)}last.`;
    const shown = thoughtNow(line).line;
    expect(shown.startsWith("word word")).toBe(true);
    expect(shown.endsWith("…")).toBe(true);
    expect(shown.match(/…/g)).toHaveLength(1);
  });

  it("finds the line without reading the whole turn back", () => {
    const long = `${"filler filler filler. ".repeat(2000)}\nThe answer is 391.`;
    expect(thoughtNow(long).line).toBe("The answer is 391.");
  });
});

describe("the heading above it", () => {
  it("is the model's own section heading, in either convention", () => {
    expect(thoughtNow("**Checking the totals**\nthe ledger agrees.")).toEqual({
      heading: "Checking the totals",
      line: "the ledger agrees.",
    });
    expect(thoughtNow("## Weighing it up\nboth options cost the same.")).toEqual({
      heading: "Weighing it up",
      line: "both options cost the same.",
    });
  });

  it("stands alone until a sentence finishes under it", () => {
    expect(thoughtNow("**Checking the totals**\n\nthe third quarter does not")).toEqual({
      heading: "Checking the totals",
      line: "",
    });
  });

  it("drops the sentence said under the last one", () => {
    const held = "**Totals**\nthe ledger agrees.\n\n**Drafting the reply**\n";
    expect(thoughtNow(held)).toEqual({ heading: "Drafting the reply", line: "" });
  });

  it("is the newest of a turn that publishes nothing but headings", () => {
    const held = "**Updating skill memory**\n\n**Checking suppression and site**\n\n**Sending";
    expect(thoughtNow(held)).toEqual({ heading: "Checking suppression and site", line: "" });
    expect(thoughtNow(`${held} outreach**`)).toEqual({ heading: "Sending outreach", line: "" });
  });

  it("is not a sentence with emphasis in it", () => {
    expect(thoughtNow("I should **check** the totals **now**.")).toEqual({
      heading: "",
      line: "I should check the totals now.",
    });
  });
});
