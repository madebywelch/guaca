import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { LIMITS } from "./limits";
import { SECTION_LABELS, SECTIONS } from "./sections";

/**
 * The `guaca` skill is the app described to its own agents, and it is only
 * useful while it is true. An agent that tells the operator to open a pane
 * that was renamed sends them hunting, so every pane the app draws has to be
 * named in it, in bold, the way the manual names places.
 */
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
    // The sentence about the screen and the sentence under a quick action both
    // name a pane, and an agent that names one the operator cannot find sends
    // them hunting. The id crosses the wire; the name is what is read.
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
    // An operator approving "Relay depth: 8 → 12" has to recognize the row
    // they would have changed by hand. `LIMIT_LABELS` in config.rs is what the
    // approval card says; `LIMITS` is what the pane says.
    const rust = read("src-tauri/src/config.rs");
    for (const field of LIMITS) {
      expect(rust, `${field.key} is labeled differently`).toContain(
        `("${field.key}", "${field.label}")`,
      );
    }
  });
});
