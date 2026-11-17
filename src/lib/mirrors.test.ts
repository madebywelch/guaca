import { readdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import { compact, money } from "../components/Spend";
import { LIMITS } from "./limits";
import { allows } from "./plugins";
import { groupNamed } from "./presence";
import { otherBuild } from "./releases";
import { eventSpec, humanGap, parseTrigger, repeats, titled } from "./routine";
import { attachedFiles, sendRecipients } from "./toolArgs";
import { readSpent, splitPluginTool, surfaceOf } from "./trail";
import {
  type AgentId,
  CATALOG,
  type Envelope,
  type Group,
  type GroupId,
  type Part,
  type PluginAccess,
  plainText,
} from "./types";

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

const AGENT: AgentId = "00000000-0000-4000-8000-000000000001";

const ENVELOPE: Envelope = {
  id: "00000000-0000-4000-8000-000000000010",
  runId: "00000000-0000-4000-8000-000000000011",
  channelId: AGENT,
  from: { kind: "human" },
  to: { kind: "agent", id: AGENT },
  parts: [],
  trust: "operator",
  hop: 0,
  expectsReply: true,
  intent: "work",
  cause: null,
  createdAt: 0,
};

const CHECKS: Record<string, Check> = {
  builds: each(([a, b]: [string, string]) => otherBuild(a, b)),
  compact: each(compact),
  "crew-name": each(({ groups, group }: { groups: Pick<Group, "id" | "name">[]; group: GroupId }) =>
    groupNamed(groups, group),
  ),
  constants: (file) => {
    expect((file as { catalog: string[] }).catalog).toEqual([...CATALOG]);
  },
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
  money: each(money),
  "plain-text": (file) => {
    for (const { input, expected, rust, web } of file as {
      input: Part[];
      expected?: string;
      rust?: string;
      web?: string;
    }[]) {
      const shared = expected !== undefined;
      expect(shared !== (rust !== undefined && web !== undefined), JSON.stringify(input)).toBe(
        true,
      );
      expect(plainText({ ...ENVELOPE, parts: input }), JSON.stringify(input)).toBe(
        shared ? expected : web,
      );
    }
  },
  "plugin-access": each(({ access, agent }: { access: PluginAccess; agent: AgentId }) =>
    allows(access, agent),
  ),
  "plugin-call": each(splitPluginTool),
  titled: each(titled),
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
