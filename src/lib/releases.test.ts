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
  shortBuild,
  skew,
  updateNotice,
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
  it("names a build by its short commit, and names nothing that is not a commit", () => {
    expect(shortBuild("")).toBe("");
    expect(shortBuild("unknown")).toBe("");
    expect(shortBuild("abc")).toBe("");
    expect(shortBuild("c15bd9a-dirty; rm")).toBe("");
    expect(shortBuild("c15bd9a".padEnd(40, "0"))).toBe("c15bd9a");
    expect(shortBuild("79961bb-dirty")).toBe("79961bb-dirty");
    expect(shortBuild(`${"c15bd9a".padEnd(40, "0")}-dirty`)).toBe("c15bd9a-dirty");
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

describe("a desktop and its host, updated separately", () => {
  const facts = (over: Partial<Parameters<typeof updateNotice>[0]> = {}) =>
    updateNotice({
      desktop: true,
      client: { version: "0.1.0", commit: "aaaaaaa" },
      health,
      release: null,
      localUpdate: null,
      ...over,
    });
  const ours = (version: string, commit = "aaaaaaa") => ({ version, commit });
  it("orders a client and its host only by stable releases", () => {
    expect(skew(ours("0.1.0"), health)).toBe("same");
    expect(skew(ours("0.2.0"), health)).toBe("hostBehind");
    expect(skew(ours("0.0.9"), health)).toBe("clientBehind");
    expect(skew(ours("0.1.0"), { ...health, version: undefined })).toBe("unknown");
    expect(skew(ours("0.1.0-dev"), health)).toBe("unknown");
    expect(skew(ours("0.1.0"), null)).toBe("unknown");
  });
  it("tells one version on two builds from one build", () => {
    // A source build carries the last release's version, so against that
    // release's host a version alone said "same" while the host was missing
    // every command added since, and the Host pane said Compatible.
    expect(skew(ours("0.1.0", "bbbbbbb"), health)).toBe("otherBuild");
    expect(skew(ours("0.1.0", "bbbbbbb-dirty"), health)).toBe("otherBuild");
    expect(skew(ours("0.1.0", "aaaaaaa-dirty"), health)).toBe("otherBuild");
    // One commit at two lengths, one dirty tree on both sides, or a side that
    // names no commit, is not a difference anyone can show.
    expect(skew(ours("0.1.0", "a".repeat(40)), health)).toBe("same");
    const dirty = { ...health, build: "aaaaaaa-dirty" };
    expect(skew(ours("0.1.0", "aaaaaaa-dirty"), dirty)).toBe("same");
    expect(skew(ours("0.1.0", ""), health)).toBe("same");
    expect(skew(ours("0.1.0"), { ...health, build: "" })).toBe("same");
    expect(skew(ours("0.1.0", "unknown"), health)).toBe("same");
  });
  it("says one version on two builds before any release news", () => {
    const notice = facts({ client: ours("0.1.0", "bbbbbbb"), release: status });
    expect(notice?.text).toBe(
      "This app and its host are different builds of Guaca 0.1.0, so features one has may be missing from the other. Run both from the same build.",
    );
    expect(notice?.key).toBe("otherBuild:aaaaaaa:bbbbbbb");
    expect(facts({ desktop: false, client: ours("0.1.0", "bbbbbbb-dirty") })).toBeNull();
  });
  it("says which side to update when they differ, before any release news", () => {
    const ahead = facts({ client: { version: "0.3.0", commit: "b" }, release: status });
    expect(ahead?.text).toBe(
      "This host runs Guaca 0.1.0 and this app is 0.3.0. Update the host to match.",
    );
    const behind = facts({ health: { ...health, version: "0.2.0" }, release: status });
    expect(behind?.text).toBe(
      "This host runs Guaca 0.2.0 and this app is 0.1.0. Update this app to match.",
    );
    expect(ahead?.key).not.toBe(behind?.key);
  });
  it("names both when a release is newer than each of them", () => {
    expect(facts({ release: status })?.text).toBe(
      "Guaca 0.2.0 is available for this app and its host.",
    );
    expect(facts({ desktop: false, release: status })?.text).toBe(
      "Host update available: Guaca 0.2.0.",
    );
  });
  it("tells a desktop about its own release even when the host cannot be ordered", () => {
    const current = { ...health, version: "0.2.0" };
    const app = { version: "0.2.0", commit: "a" };
    expect(facts({ client: app, health: current, release: status })).toBeNull();
    const release = { ...status, latest: { ...status.latest!, version: "0.3.0" } };
    expect(facts({ client: app, health: current, release })?.text).toBe(
      "Guaca 0.3.0 is available for this app and its host.",
    );
    // A source-built host is never claimed to be behind, and the app still is.
    const source = { ...current, release: false };
    expect(facts({ client: app, health: source, release })?.text).toBe(
      "Guaca 0.3.0 is available for this app.",
    );
  });
  it("puts a stale page first, and never judges a development page stale", () => {
    const page = { desktop: false, client: { version: "0.1.0", commit: "bbbbbbb" } };
    expect(facts({ ...page, release: status })?.key).toBe("page:aaaaaaa");
    const dirty = { desktop: false, client: { version: "0.1.0", commit: "bbbbbbb-dirty" } };
    expect(facts(dirty)).toBeNull();
  });
  it("offers a managed host's own image only when nothing more specific applies", () => {
    expect(facts({ localUpdate: "guacad:new" })?.text).toBe("Host update available.");
    expect(facts({ localUpdate: null })).toBeNull();
  });
});

describe("a box that follows main", () => {
  const tip = "d".repeat(40);
  const main: ReleaseStatus = {
    ...status,
    channel: "main",
    latest: {
      ...status.latest!,
      channel: "main",
      version: "0.1.0",
      commit: tip,
      notes: `https://github.com/madebywelch/guaca/commit/${tip}`,
    },
  };
  it("refuses a build of main that links anything but its commit, or claims the other channel", () => {
    for (const bad of [
      { ...main, latest: { ...main.latest!, notes: status.latest!.notes } },
      { ...main, channel: "release" },
      { ...main, channel: undefined },
      { ...status, channel: "main" },
      { ...main, channel: "nightly" },
      { ...main, latest: { ...main.latest!, channel: "nightly" } },
    ])
      expect(() => parseReleaseStatus(bad)).toThrow();
    expect(parseReleaseStatus(main)).toEqual(main);
    expect(parseReleaseStatus({ ...status, channel: "release" }).latest?.version).toBe("0.2.0");
  });
  it("is behind when its commit is not the tip, whatever the versions say", () => {
    expect(available({ ...health, build: tip.slice(0, 7) }, main)).toBe(false);
    expect(available({ ...health, build: tip }, main)).toBe(false);
    expect(available({ ...health, build: `${tip.slice(0, 7)}-dirty` }, main)).toBe(true);
    expect(available({ ...health, build: "aaaaaaa", release: false }, main)).toBe(true);
    expect(available({ ...health, build: "", version: "9.9.9" }, main)).toBe(true);
  });
  it("names the build it would install", () => {
    const notice = updateNotice({
      desktop: false,
      client: { version: "0.1.0", commit: "aaaaaaa" },
      health,
      release: main,
      localUpdate: null,
    });
    expect(notice).toEqual({
      key: `host:main:${tip}`,
      text: "Host update available: main at ddddddd.",
    });
  });
});
