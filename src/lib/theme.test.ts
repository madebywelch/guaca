/**
 * The gates a theme is held to, and the stylesheet held to the theme.
 *
 * Nothing here says amber is right or that the columns should be a particular
 * gray. It says every combination an operator can pick is readable, that the
 * signals stay apart for somebody who cannot tell red from green, and that the
 * values `styles.css` paints before any script runs are the ones the solver
 * would have written anyway. Colorblind separation and contrast are exactly
 * the things nobody can check by looking, which is why none of it is left to
 * a screenshot.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import {
  ATTENTIONS,
  type AttentionKey,
  CONTRASTS,
  DEFAULT_THEME,
  GRAYS,
  type GrayKey,
  measure,
  THEME_TOKENS,
  type Theme,
  type Tokens,
  themeCss,
  themeTokens,
} from "./theme";

const css = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");

/** What one block of a stylesheet declares, by name. */
function declared(source: string, selector: string): Map<string, string> {
  const block = source.match(
    new RegExp(`^${selector.replace(/[[\]"=]/g, "\\$&")} \\{\\n[\\s\\S]*?\\n\\}`, "m"),
  );
  if (!block) throw new Error(`no ${selector} block`);
  return new Map([...block[0].matchAll(/^\s{2}(--[a-z-]+): (.+);$/gm)].map((m) => [m[1]!, m[2]!]));
}

const PAPER = ":root";
const INK = ':root[data-surface="dark"]';

/** Every combination the Appearance pane can reach. */
const THEMES: Theme[] = (Object.keys(GRAYS) as GrayKey[]).flatMap((grays) =>
  (Object.keys(ATTENTIONS) as AttentionKey[]).flatMap((attention) =>
    CONTRASTS.map((contrast) => ({ grays, attention, contrast })),
  ),
);

/** What failed, as sentences a failure message can print. */
function failures(tokens: Tokens, level: Theme["contrast"]): string[] {
  return measure(tokens, level)
    .filter((check) => check.value < check.target)
    .map((check) => `${check.name}: ${check.value.toFixed(2)}, needs ${check.target}`);
}

describe("the gates", () => {
  /**
   * The stylesheet as it stood before themes, in today's names.
   *
   * Kept as the failure the gates exist for. Hints at 3.2 to 1, a control's
   * border at 1.3, and "answer me" 2.6 from "failed" under deuteranopia, which
   * is one color: every one of these shipped, and every one passed the suites
   * of the day, because none of them is visible to the person who picked it.
   */
  const SHIPPED: Tokens = {
    "--rail-ground": "#f5f3ee",
    "--rail-raised": "#ffffff",
    "--rail-edge": "#e5e2da",
    "--rail-text": "#0b0b0a",
    "--rail-muted": "#54524d",
    "--rail-wire": "#ded9d0",
    "--rail-sunken": "#eae7df",
    "--grail-ground": "#eceae2",
    "--ground": "#ffffff",
    "--raised": "#ffffff",
    "--sunken": "#f4f3f0",
    "--edge": "#e2e0da",
    "--edge-control": "#e2e0da",
    "--text": "#0b0b0a",
    "--muted": "#54524d",
    "--faint": "#8a877f",
    "--attention": "#b4530a",
    "--attention-wash": "#fdeed9",
    "--attention-fill": "#b4530a",
    "--on-attention-fill": "#ffffff",
    "--system": "#2f4858",
    "--system-wash": "#e9eff3",
    "--ok": "#14713f",
    "--danger": "#c1271b",
    "--danger-fill": "#c1271b",
    "--on-danger-fill": "#ffffff",
  };

  it("fail the stylesheet as it shipped, on exactly what was wrong with it", () => {
    const failed = measure(SHIPPED, "standard")
      .filter((check) => check.value < check.target)
      .map((check) => check.name);

    expect(failed).toEqual(
      expect.arrayContaining([
        "faint where hints are written",
        "a control's border",
        "attention fill beside danger fill, colorblind",
        "attention fill beside ok, colorblind",
      ]),
    );
  });

  it.each(
    THEMES.flatMap((theme) =>
      (["light", "dark"] as const).map((surface) => [theme, surface] as const),
    ),
  )("pass %o on %s", (theme, surface) => {
    expect(failures(themeTokens(theme, surface), theme.contrast)).toEqual([]);
  });
});

describe("what a theme can and cannot move", () => {
  it("keeps the page on paper white, whatever the grays", () => {
    // The page is the only white thing, and a tint there is the reading
    // surface picking up a color every agent's color then competes with.
    for (const grays of Object.keys(GRAYS) as GrayKey[]) {
      expect(themeTokens({ ...DEFAULT_THEME, grays }, "light")["--ground"]).toBe("#ffffff");
    }
  });

  it("draws a fill the same on both surfaces, text color included", () => {
    // A fill carries its own text color precisely so it does not have to
    // agree with the page. One that moved with the surface would be a badge
    // that is a different color in a dark room for no reason.
    for (const theme of THEMES) {
      const light = themeTokens(theme, "light");
      const dark = themeTokens(theme, "dark");
      for (const name of [
        "--attention-fill",
        "--on-attention-fill",
        "--danger-fill",
        "--on-danger-fill",
      ] as const) {
        expect(dark[name], `${name} in ${JSON.stringify(theme)}`).toBe(light[name]);
      }
    }
  });

  it("writes every token it owns into both blocks, and nothing else", () => {
    const written = themeCss({ grays: "warm", attention: "blue", contrast: "more" });
    for (const selector of [PAPER, INK]) {
      expect([...declared(written, selector).keys()]).toEqual([...THEME_TOKENS]);
    }
  });
});

describe("the stylesheet's own values", () => {
  /**
   * `styles.css` is the first paint and the fallback, and the default theme is
   * what an operator who changed nothing sees. If the two disagree, every
   * launch paints one set of colors and then snaps to the other, and the set
   * in the stylesheet is one nobody measured.
   */
  it.each([
    ["light", PAPER],
    ["dark", INK],
  ] as const)("declares the default theme on %s", (surface, selector) => {
    const block = declared(css, selector);
    const solved = themeTokens(DEFAULT_THEME, surface);
    for (const name of THEME_TOKENS) {
      expect(block.get(name), `${name} in ${selector}`).toBe(solved[name]);
    }
  });
});
