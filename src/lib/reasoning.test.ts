import { describe, expect, it } from "vitest";

import { keepThought, thoughtLine } from "./reasoning";

describe("keeping a thought", () => {
  it("appends what arrives", () => {
    expect(keepThought(undefined, "I need")).toBe("I need");
    expect(keepThought("I need", " the total")).toBe("I need the total");
  });

  it("keeps the end rather than the beginning", () => {
    // The head is the first thing the model thought and never changes again.
    const long = "a".repeat(300);
    const held = keepThought(long, "END");
    expect(held.endsWith("END")).toBe(true);
    expect(held.length).toBeLessThanOrEqual(240);
  });

  it("keeps the newlines, because they are where a line starts", () => {
    // Reduced to the last line on the way in, "Checking totals\n\n" plus "Now
    // the sum" concatenated into one run-on line with no boundary left to find.
    const held = keepThought(keepThought("", "Checking totals\n\n"), "Now the sum");
    expect(thoughtLine(held)).toBe("Now the sum");
  });
});

describe("the line drawn from it", () => {
  it("is the last line with anything on it", () => {
    expect(thoughtLine("first thought\n\nsecond thought")).toBe("second thought");
  });

  it("holds the previous line while the next one is still blank", () => {
    // A delta that is only a paragraph break must not blank the line.
    expect(thoughtLine("Checking totals\n\n")).toBe("Checking totals");
  });

  it("takes the marks off a heading rather than drawing them", () => {
    expect(thoughtLine("**Checking the totals**")).toBe("Checking the totals");
    expect(thoughtLine("## Weighing it up")).toBe("Weighing it up");
    expect(thoughtLine("running `update_notes` next")).toBe("running update_notes next");
  });

  it("leaves an identifier alone", () => {
    // Stripping every markdown character would have made this "updatenotes".
    expect(thoughtLine("call update_notes with the whole file")).toBe(
      "call update_notes with the whole file",
    );
  });

  it("scrolls a long line from the right, marked once however long it gets", () => {
    const line = `${"word ".repeat(60)}newest`;
    const shown = thoughtLine(line);
    expect(shown.endsWith("newest")).toBe(true);
    expect(shown.startsWith("…")).toBe(true);
    // Re-truncating an already-truncated line must not stack ellipses.
    expect(shown.match(/…/g)).toHaveLength(1);
  });

  it("is empty when there is nothing to say", () => {
    expect(thoughtLine(undefined)).toBe("");
    expect(thoughtLine("\n\n  \n")).toBe("");
  });
});
