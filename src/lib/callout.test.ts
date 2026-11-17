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
    expect(readCallout("[!Important]\nx")?.register).toBe("asks");
    expect(readCallout("[!note]\nx")?.register).toBe("aside");
  });

  it("leaves nothing behind when the marker had the line to itself", () => {
    expect(readCallout("[!NOTE]\n")).toEqual({ register: "aside", rest: "" });
    expect(readCallout("[!NOTE]")).toEqual({ register: "aside", rest: "" });
    expect(readCallout("[!NOTE]  \nx")).toEqual({ register: "aside", rest: "x" });
  });

  it("keeps the rest of a quote written as one wrapped paragraph", () => {
    expect(readCallout("[!IMPORTANT]\nfirst\nsecond")).toEqual({
      register: "asks",
      rest: "first\nsecond",
    });
  });
});

describe("a quote that is only a quote", () => {
  it("is one when the marker is a word this app does not draw", () => {
    expect(readCallout("[!DANGER]\nx")).toBeNull();
    expect(readCallout("[!]\nx")).toBeNull();
  });

  it("is one when the marker opens a sentence rather than a line", () => {
    expect(readCallout("[!IMPORTANT] ship it")).toBeNull();
  });

  it("is one when the words start before the marker does", () => {
    expect(readCallout("Read this: [!WARNING]\nx")).toBeNull();
  });
});

describe("the label", () => {
  it("says what the box is for, in the app's own words", () => {
    expect(LABEL.asks).toBe("Needs you");
    expect(LABEL.aside).toBe("Note");
  });
});
