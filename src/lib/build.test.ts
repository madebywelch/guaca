import { describe, expect, it } from "vitest";

import { buildLabel, COMMIT } from "./build";

describe("buildLabel", () => {
  it("names the commit the build was made from", () => {
    expect(buildLabel("9f2c1a4")).toBe("Version 9f2c1a4");
  });

  it("carries the mark that says the tree was not clean", () => {
    expect(buildLabel("9f2c1a4-dirty")).toBe("Version 9f2c1a4-dirty");
  });

  it("says a dash when there was no repository to read", () => {
    expect(buildLabel("")).toBe("Version —");
  });

  it("is built from a commit, which is what the define is for", () => {
    // Guards the wiring rather than the value: an undefined `__COMMIT__` is a
    // config that stopped substituting, and it would draw as a dash forever
    // while every other test in this file still passed.
    expect(typeof COMMIT).toBe("string");
    expect(COMMIT === "" || /^[0-9a-f]{7,}(-dirty)?$/.test(COMMIT)).toBe(true);
  });
});
