import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { ROOT_PX } from "./appearance";
import { DEFAULT_THEME, themeTokens } from "./theme";
import { BAR_HEIGHT_PX, BAR_TYPE_PX, barLook, dressed } from "./widget";

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

  it("is handed the bar's face as a data: URL, because the frame may load no other", () => {
    const framed = dressed("", barLook(DEFAULT_THEME, "light", 100), {
      family: '"Inter Variable"',
      stack: '"Inter Variable", system-ui, sans-serif',
      src: "data:font/woff2;base64,AAAA",
    });
    expect(framed).toContain('@font-face { font-family: "Inter Variable";');
    expect(framed).toContain('src: url("data:font/woff2;base64,AAAA") format("woff2-variations")');
    expect(framed).toContain('--guaca-font: "Inter Variable", system-ui, sans-serif;');
  });

  it("draws a face the machine already has without an @font-face", () => {
    const framed = dressed("", barLook(DEFAULT_THEME, "light", 100, "system"));
    expect(framed).not.toContain("@font-face");
    expect(framed).toContain("--guaca-font: system-ui, -apple-system");
  });

  it("ends a line too long for the frame with an ellipsis rather than a cut letter", () => {
    const look = barLook(DEFAULT_THEME, "light", 100);
    const framed = dressed("<b>13.5</b><span>4 steps checked</span>", look);
    expect(framed).toContain("text-overflow: ellipsis");
    expect(framed).toContain("white-space: nowrap");
    expect(framed).toContain(`--guaca-height: ${look.heightPx}px;`);
    expect(framed).toContain(":where(body > * + *)");
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

  it("draw a condensed view in the bar's own height", () => {
    // The one line is set to the frame's height so it is centered in the frame
    // and a line too long for it can end in an ellipsis.
    expect(rem("--control-h-sm") * ROOT_PX).toBe(BAR_HEIGHT_PX);
  });
});
