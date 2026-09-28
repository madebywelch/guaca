import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { ROOT_PX } from "./appearance";
import { DEFAULT_THEME, themeTokens } from "./theme";
import { BAR_TYPE_PX, barLook, dressed } from "./widget";

const read = (path: string) => readFileSync(join(process.cwd(), path), "utf8");

describe("a condensed view's frame", () => {
  it("is handed the bar's colors, surface and size ahead of the page", () => {
    const look = barLook(DEFAULT_THEME, "dark", 125);
    const framed = dressed("<b>3 open</b>", look);
    const dark = themeTokens(DEFAULT_THEME, "dark");
    expect(framed.endsWith("<b>3 open</b>")).toBe(true);
    expect(framed).toContain(`--guaca-text: ${dark["--text"]};`);
    expect(framed).toContain(`--guaca-attention: ${dark["--attention"]};`);
    expect(framed).toContain("color-scheme: dark;");
    expect(framed).toContain(`--guaca-size: ${(BAR_TYPE_PX * 125) / 100}px;`);
  });

  it("sets its defaults with no specificity, so the page's own rules win", () => {
    // A default that outranked `body { color: red }` would be the bar deciding
    // what an agent's page looks like.
    const framed = dressed("", barLook(DEFAULT_THEME, "light", 100));
    const rules = framed.match(/\}([^{}]+)\{/g) ?? [];
    for (const rule of rules) expect(rule).toMatch(/:where\(/);
  });
});

describe("the sizes an agent is told", () => {
  const css = read("src/styles.css");
  const rem = (name: string) => Number(css.match(new RegExp(`${name}: ([\\d.]+)rem;`))?.[1]);

  it("are the sizes the bar draws", () => {
    // The tool description is the only place an agent learns how big the strip
    // is, in pixels. Change a width here and that sentence is wrong.
    const tools = read("src-tauri/src/llm/tools.rs");
    const said = tools.match(/about (\d+) pixels wide when\s*\\?\s*narrow and (\d+) when wide/);
    expect(said, "the artifact tool states both widths").not.toBeNull();
    expect(rem("--widget-narrow") * ROOT_PX).toBe(Number(said?.[1]));
    expect(rem("--widget-wide") * ROOT_PX).toBe(Number(said?.[2]));
  });

  it("draw a condensed view in the bar's own type size", () => {
    expect(css).toMatch(/\.statusbar \{[^}]*font-size: var\(--type-small\);/);
    expect(rem("--type-small") * ROOT_PX).toBe(BAR_TYPE_PX);
  });
});
