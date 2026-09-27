import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

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
    ["Settings", "src/components/SettingsDialog.tsx"],
    ["a crew's settings", "src/components/GroupEditor.tsx"],
  ])("names every pane in %s", (_where, path) => {
    const found = labels(read(path));
    expect(found.length).toBeGreaterThan(5);
    const missing = found.filter((label) => !manual.includes(`**${label}**`));
    expect(missing, "panes the manual does not describe").toEqual([]);
  });
});
