import { describe, expect, it } from "vitest";

import { LABEL, readCallout } from "./callout";

describe("a quote that opens with a marker", () => {
  it.each([
    ["IMPORTANT", "asks"],
    ["WARNING", "asks"],
    ["CAUTION", "asks"],
    ["NOTE", "aside"],
    ["TIP", "aside"],
  ])("%s opens the %s box", (marker, register) => {
    expect(readCallout(`[!${marker}]\nRotate the staging key.`)).toEqual({
      register,
      rest: "Rotate the staging key.",
    });
  });

  it("takes the marker however the model cased it", () => {
    // A model writes these from memory, and the ones it has read were written
    // by people. Refusing `[!Note]` is a box that silently does not appear.
    expect(readCallout("[!Important]\nx")?.register).toBe("asks");
    expect(readCallout("[!note]\nx")?.register).toBe("aside");
  });

  it("leaves nothing behind when the marker had the line to itself", () => {
    expect(readCallout("[!NOTE]\n")).toEqual({ register: "aside", rest: "" });
    expect(readCallout("[!NOTE]")).toEqual({ register: "aside", rest: "" });
    // Trailing blanks a model left and no operator can see.
    expect(readCallout("[!NOTE]  \nx")).toEqual({ register: "aside", rest: "x" });
  });

  it("keeps the rest of a quote written as one wrapped paragraph", () => {
    // Valid markdown and the same meaning: the soft break is in the text.
    expect(readCallout("[!IMPORTANT]\nfirst\nsecond")).toEqual({
      register: "asks",
      rest: "first\nsecond",
    });
  });
});

describe("a quote that is only a quote", () => {
  it("is one when the marker is a word this app does not draw", () => {
    // The closed set is the whole discipline: an unknown marker drawn as a box
    // is a box whose color nobody chose.
    expect(readCallout("[!DANGER]\nx")).toBeNull();
    expect(readCallout("[!]\nx")).toBeNull();
  });

  it("is one when the marker opens a sentence rather than a line", () => {
    // `[!IMPORTANT] ship it` is prose that begins with a bracket, and a box
    // round it would eat the two words after the marker.
    expect(readCallout("[!IMPORTANT] ship it")).toBeNull();
  });

  it("is one when the words start before the marker does", () => {
    expect(readCallout("Read this: [!WARNING]\nx")).toBeNull();
  });
});

describe("the label", () => {
  /**
   * Five markers, two words. What the operator needs off the box is whether it
   * is for them, and *Needs you* is the sentence the rail already says about an
   * agent that is waiting on somebody.
   */
  it("says what the box is for, in the app's own words", () => {
    expect(LABEL.asks).toBe("Needs you");
    expect(LABEL.aside).toBe("Note");
  });
});
