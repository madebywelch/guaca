import { describe, expect, it } from "vitest";

import { buildLabel, COMMIT, VERSION } from "./build";

describe("buildLabel", () => {
  it("leads with the version a release is published under", () => {
    expect(buildLabel("0.2.0", "9f2c1a4")).toBe("Version 0.2.0 (9f2c1a4)");
  });

  it("carries the mark that says the tree was not clean", () => {
    expect(buildLabel("0.2.0", "9f2c1a4-dirty")).toBe("Version 0.2.0 (9f2c1a4-dirty)");
  });

  it("says the version alone where there was no repository to read", () => {
    expect(buildLabel("0.2.0", "")).toBe("Version 0.2.0");
  });

  it("falls back to the commit where there is no version to name", () => {
    expect(buildLabel("", "9f2c1a4")).toBe("Version 9f2c1a4");
  });

  it("says a dash when there was nothing to read", () => {
    expect(buildLabel("", "")).toBe("Version unknown");
  });

  it("is built from a version and a commit, which is what the defines are for", () => {
    // Guards the wiring rather than the value: an undefined `__VERSION__` is a
    // config that stopped substituting, and every other test here would still
    // pass while About drew a dash forever.
    expect(typeof VERSION).toBe("string");
    expect(VERSION).toMatch(/^\d+\.\d+\.\d+/);
    expect(typeof COMMIT).toBe("string");
    expect(COMMIT === "" || /^[0-9a-f]{7,}(-dirty)?$/.test(COMMIT)).toBe(true);
  });
});
