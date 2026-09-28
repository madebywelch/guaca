import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import type { HostOperation } from "./host";
import { HOST_STEPS, hostProgress, REBUILD_STEPS, rebuildProgress } from "./updating";

const read = (path: string) => readFileSync(join(process.cwd(), path), "utf8");

const journal = (stage: string, targetImage = "new", targetVersion = "0.2.0"): HostOperation => ({
  stage,
  backup: null,
  previousImage: "old",
  targetImage,
  targetVersion,
  error: null,
});
const target = { image: "new" };

describe("the steps are the ones the programs say", () => {
  it("names a host update's steps as the journal writes them", () => {
    const stages = Object.fromEntries(
      [...read("src-tauri/src/host.rs").matchAll(/Stage::(\w+) => "([^"]+)"/g)].map((m) => [
        m[1],
        m[2],
      ]),
    );
    expect(HOST_STEPS).toEqual(
      ["Downloading", "Stopping", "BackingUp", "Starting", "Verifying"].map((s) => stages[s]),
    );
    // The two stages read by name rather than by place.
    expect(stages.Updated).toBe("Host updated");
    expect(stages.Restoring).toBe("Restoring previous version");
  });

  it("names a rebuild's steps as install.sh announces them, up to quitting this app", () => {
    const announced = [...read("scripts/install.sh").matchAll(/^\s*step "([^"]+)"/gm)].map(
      (m) => m[1],
    );
    const quit = announced.indexOf("Quitting the running Guaca");
    expect(quit).toBeGreaterThan(0);
    expect(REBUILD_STEPS).toEqual(announced.slice(0, quit + 1));
  });
});

describe("a host update", () => {
  it("never shows the last update's ending as this one's", () => {
    for (const stage of [
      "Host updated",
      "Update canceled",
      "Previous version restored",
      "Recovery needed",
    ]) {
      expect(hostProgress(journal(stage, "older", "0.1.0"), target, false)).toMatchObject({
        done: 0,
        now: 1,
        label: "Starting update",
      });
    }
  });

  it("starts a retry from the beginning, not from where the last attempt stopped", () => {
    expect(hostProgress(journal("Update canceled"), target, false)).toMatchObject({
      done: 0,
      label: "Starting update",
    });
  });

  it("starts from the beginning before there is a journal at all", () => {
    expect(hostProgress(null, target, false)).toMatchObject({ done: 0, now: 1 });
  });

  it("is at the step the journal names, with every step before it finished", () => {
    HOST_STEPS.forEach((stage, at) => {
      expect(hostProgress(journal(stage), target, false)).toEqual({
        steps: HOST_STEPS,
        done: at,
        now: 1,
        label: stage,
      });
    });
  });

  it("is finished only when this update's journal says so", () => {
    expect(hostProgress(journal("Host updated"), target, false)).toMatchObject({
      done: HOST_STEPS.length,
      now: 0,
    });
    expect(
      hostProgress(journal("Host updated", "", "0.2.0"), { version: "0.2.0" }, false),
    ).toMatchObject({ done: HOST_STEPS.length });
  });

  it("does not say which step a restore is past", () => {
    expect(hostProgress(journal("Restoring previous version"), target, false)).toMatchObject({
      done: 0,
      now: HOST_STEPS.length,
      label: "Restoring previous version",
    });
  });

  it("spans the steps a box's host is silent through, from the last one it was seen at", () => {
    const stopping = HOST_STEPS.indexOf("Stopping host");
    const starting = HOST_STEPS.indexOf("Starting updated host");
    // Last seen downloading, now silent: it has at least been stopped.
    expect(hostProgress(journal("Downloading update"), target, true)).toMatchObject({
      done: stopping,
      now: starting - stopping + 1,
      label: "Waiting for the host to answer",
    });
    expect(hostProgress(journal("Backing up workspace"), target, true)).toMatchObject({
      done: HOST_STEPS.indexOf("Backing up workspace"),
      now: 2,
    });
    // Silent again after the new host answered once: it is past starting.
    expect(hostProgress(journal("Verifying host"), target, true)).toMatchObject({
      done: HOST_STEPS.indexOf("Verifying host"),
      now: 1,
    });
  });
});

describe("a rebuild", () => {
  it("starts from the beginning before the script has said anything", () => {
    expect(rebuildProgress(null)).toMatchObject({ done: 0, now: 1, label: "Starting the rebuild" });
  });

  it("is at the step the script last announced", () => {
    expect(rebuildProgress("Building")).toEqual({
      steps: REBUILD_STEPS,
      done: REBUILD_STEPS.indexOf("Building"),
      now: 1,
      label: "Building",
    });
  });

  it("says a step it does not know without placing it", () => {
    expect(rebuildProgress("Fetching dependencies")).toMatchObject({
      done: 0,
      now: REBUILD_STEPS.length,
      label: "Fetching dependencies",
    });
  });
});
