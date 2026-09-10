import { describe, expect, it } from "vitest";
import {
  available,
  compareVersions,
  compatibility,
  type Health,
  parseHealth,
  parseReleaseStatus,
  type ReleaseStatus,
  sameBuild,
} from "./releases";

const health: Health = {
  service: "guacad",
  build: "aaaaaaa",
  version: "0.1.0",
  release: true,
  apiGeneration: 1,
};
const status: ReleaseStatus = {
  automatic: true,
  checkedAt: "2026-09-10T00:00:00Z",
  error: null,
  latest: {
    schema: 1,
    version: "0.2.0",
    commit: "b".repeat(40),
    image: `ghcr.io/madebywelch/guaca/guacad@sha256:${"c".repeat(64)}`,
    apiGeneration: 1,
    clientMinimum: 1,
    clientMaximum: 1,
    notes: "https://github.com/madebywelch/guaca/releases/tag/v0.2.0",
  },
};
describe("release detection", () => {
  it("finds an outdated host even when its browser has the same commit", () => {
    expect(sameBuild(health.build, health.build)).toBe(true);
    expect(available(health, status)).toBe(true);
  });
  it("never orders development builds, prereleases, or invalid versions", () => {
    expect(available({ ...health, release: false }, status)).toBe(false);
    for (const value of ["0.2.0-beta.1", "0.2.0+dirty", "01.2.0", "99999999999999999.0.0"])
      expect(compareVersions(value, "0.1.0")).toBeNull();
    expect(compareVersions("0.10.0", "0.2.0")).toBe(1);
    expect(available({ ...health, version: "0.3.0" }, status)).toBe(false);
  });
  it("keeps compatibility independent from release freshness", () => {
    expect(compatibility(health)).toBe("compatible");
    expect(compatibility({ ...health, apiGeneration: 2 })).toBe("clientOld");
    expect(compatibility(parseHealth({ service: "guacad", build: "aaaaaaa" }))).toBe("unknown");
    for (const apiGeneration of [0, -1, NaN, 1.5])
      expect(compatibility({ ...health, apiGeneration })).toBe("unknown");
  });
  it("accepts different lengths of the same clean commit", () => {
    expect(sameBuild("abcdef1", "abcdef123456")).toBe(true);
    expect(sameBuild("abcdef1-dirty", "abcdef1")).toBe(false);
    expect(sameBuild("", "")).toBe(false);
  });
  it("does not render untrusted release links or install targets", () => {
    expect(parseReleaseStatus(status)).toEqual(status);
    for (const bad of [
      { notes: "javascript:alert(1)" },
      { image: "attacker/image:latest" },
      { clientMaximum: 0 },
    ]) {
      expect(() =>
        parseReleaseStatus({ ...status, latest: { ...status.latest, ...bad } }),
      ).toThrow();
    }
    expect(() => parseHealth({ service: "another-service" })).toThrow();
    expect(() => parseReleaseStatus({})).toThrow();
  });
});
