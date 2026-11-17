import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import { LIMITS } from "./limits";
import { allows } from "./plugins";
import { eventSpec, humanGap, parseTrigger, repeats } from "./routine";
import { attachedFiles, sendRecipients } from "./toolArgs";
import { readSpent, surfaceOf } from "./trail";
import { type AgentId, type PluginAccess } from "./types";

const MIRRORS = resolve(__dirname, "mirrors");

interface Case<I, E> {
  input: I;
  expected: E;
}

type Check = (file: unknown) => void;

function each<I, E>(actual: (input: I) => E): Check {
  return (file) => {
    for (const { input, expected } of file as Case<I, E>[]) {
      expect(actual(input), JSON.stringify(input)).toEqual(expected);
    }
  };
}

const CHECKS: Record<string, Check> = {
  credentials: (file) => {
    for (const { input, expected } of file as Case<
      { summary: string },
      { written: string; spent: string[] }
    >[]) {
      expect(readSpent(expected.written)).toEqual({ spent: expected.spent, rest: input.summary });
    }
  },
  "event-spec": each(({ service, topic }: { service: string; topic: string }) =>
    eventSpec(service, topic),
  ),
  "human-gap": each(humanGap),
  limits: (file) => {
    expect(LIMITS.map(({ key, min, max }) => ({ input: key, expected: { min, max } }))).toEqual(
      file,
    );
  },
  "plugin-access": each(({ access, agent }: { access: PluginAccess; agent: AgentId }) =>
    allows(access, agent),
  ),
  "tool-names": each(({ tool, args }: { tool: string; args: unknown }) =>
    tool === "attach_file" ? attachedFiles(args) : sendRecipients(args),
  ),
  "tool-surfaces": each(surfaceOf),
  trigger: each((spec: string) => {
    const trigger = parseTrigger(spec);
    return trigger.kind === "unknown" ? null : { trigger, repeats: repeats(spec) };
  }),
};

describe("vectors shared with the Rust suite", () => {
  it("has a check here for every vector file, and a file for every check", () => {
    const files = readdirSync(MIRRORS)
      .filter((name) => name.endsWith(".json"))
      .map((name) => name.slice(0, -".json".length));
    expect(files.sort()).toEqual(Object.keys(CHECKS).sort());
  });

  for (const [name, check] of Object.entries(CHECKS)) {
    it(`holds every vector in ${name}.json`, () => {
      check(JSON.parse(readFileSync(resolve(MIRRORS, `${name}.json`), "utf8")));
    });
  }
});
