import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { LIMITS } from "./limits";
import { SECTION_LABELS, SECTIONS } from "./sections";

const read = (path: string) => readFileSync(join(process.cwd(), path), "utf8");
const manual = read("src-tauri/skills/guaca/SKILL.md");

function labels(source: string): string[] {
  const block = source.match(/const SECTION_LABELS[^{]*\{([\s\S]*?)\n\};/);
  if (!block) throw new Error("no SECTION_LABELS block");
  return [...block[1]!.matchAll(/:\s*"([^"]+)"/g)].map((m) => m[1]!);
}

describe("the guaca skill", () => {
  it.each([
    ["Settings", "src/lib/sections.ts"],
    ["a group's settings", "src/components/GroupEditor.tsx"],
  ])("names every pane in %s", (_where, path) => {
    const found = labels(read(path));
    expect(found.length).toBeGreaterThan(5);
    const missing = found.filter((label) => !manual.includes(`**${label}**`));
    expect(missing, "panes the manual does not describe").toEqual([]);
  });
});

describe("the name an agent gives a pane", () => {
  it("is the one Settings draws", () => {
    const rust = read("src-tauri/src/domain/view.rs");
    for (const [id, label] of Object.entries(SECTION_LABELS)) {
      expect(rust, `${id} is named differently`).toContain(`"${id}" => "${label}"`);
    }
    const named = [...rust.matchAll(/"[a-z-]+" => "[^"]+",/g)].length;
    expect(named, "the table names a pane the app does not draw").toBe(SECTIONS.length);
  });
});

describe("the words an agent is shown for a change", () => {
  it("are the words the limits pane uses", () => {
    const rust = read("src-tauri/src/config.rs");
    for (const field of LIMITS) {
      expect(rust, `${field.key} is labeled differently`).toContain(
        `("${field.key}", "${field.label}")`,
      );
    }
  });
});
