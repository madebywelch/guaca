import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { contrast, distance, linear, oklab } from "./color";

import {
  ATTENTIONS,
  type AttentionKey,
  CONTRASTS,
  DEFAULT_THEME,
  GRAYS,
  type GrayKey,
  measure,
  TEMPERATURE_KEYS,
  THEME_PRESETS,
  THEME_TOKENS,
  type Theme,
  type Tokens,
  themeCss,
  themeTokens,
} from "./theme";

const css = readFileSync(join(process.cwd(), "src/styles.css"), "utf8");

function declared(source: string, selector: string): Map<string, string> {
  const block = source.match(
    new RegExp(`^${selector.replace(/[[\]"=]/g, "\\$&")} \\{\\n[\\s\\S]*?\\n\\}`, "m"),
  );
  if (!block) throw new Error(`no ${selector} block`);
  return new Map([...block[0].matchAll(/^\s{2}(--[a-z-]+): (.+);$/gm)].map((m) => [m[1]!, m[2]!]));
}

const PAPER = ":root";
const INK = ':root[data-surface="dark"]';

const THEMES: Theme[] = (Object.keys(GRAYS) as GrayKey[]).flatMap((grays) =>
  (Object.keys(ATTENTIONS) as AttentionKey[]).flatMap((attention) =>
    CONTRASTS.flatMap((contrast) =>
      TEMPERATURE_KEYS.map((temperature) => ({ grays, attention, contrast, temperature })),
    ),
  ),
);

function failures(tokens: Tokens, level: Theme["contrast"]): string[] {
  return measure(tokens, level)
    .filter((check) => check.value < check.target)
    .map((check) => `${check.name}: ${check.value.toFixed(2)}, needs ${check.target}`);
}

describe("the gates", () => {
  it("holds every preset to the contrast gates on both surfaces", () => {
    for (const theme of Object.values(THEME_PRESETS)) {
      for (const surface of ["light", "dark"] as const) {
        const tokens = themeTokens(theme, surface);
        expect(failures(tokens, theme.contrast)).toEqual([]);
        expect(contrast(tokens["--edge-control"], tokens["--raised"])).toBeGreaterThanOrEqual(3);
        expect(contrast(tokens["--muted"], tokens["--raised"])).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("gives Violet its own system accent without spending attention", () => {
    for (const surface of ["light", "dark"] as const) {
      const violet = themeTokens(THEME_PRESETS.Violet!, surface);
      const blue = themeTokens({ ...THEME_PRESETS.Violet!, grays: "slate" }, surface);
      expect(violet["--system"]).not.toBe(blue["--system"]);
      expect(violet["--system-wash"]).not.toBe(blue["--system-wash"]);
      expect(violet["--attention-fill"]).toBe(blue["--attention-fill"]);
    }
  });

  it("keeps the preset surfaces and action accents distinct", () => {
    for (const surface of ["light", "dark"] as const) {
      const presets = Object.values(THEME_PRESETS).map((theme) => themeTokens(theme, surface));
      for (let i = 0; i < presets.length; i++) {
        for (let j = i + 1; j < presets.length; j++) {
          expect(
            distance(presets[i]!["--rail-ground"], presets[j]!["--rail-ground"]),
          ).toBeGreaterThan(1);
          expect(
            distance(presets[i]!["--action-fill"], presets[j]!["--action-fill"]),
          ).toBeGreaterThan(4);
        }
      }
      const field = presets[1]!;
      const violet = presets[2]!;
      const [, fieldA, fieldB] = oklab(linear(field["--system"]));
      const [, violetA, violetB] = oklab(linear(violet["--system"]));
      expect(fieldA).toBeLessThan(0);
      expect(fieldB).toBeLessThan(0);
      expect(violetA).toBeGreaterThan(0.04);
      expect(violetB).toBeLessThan(-0.04);
    }
  });

  it("keeps selected rail text readable over the translucent system color", () => {
    for (const theme of Object.values(THEME_PRESETS)) {
      for (const surface of ["light", "dark"] as const) {
        const tokens = themeTokens(theme, surface);
        const mix = (a: string, b: string) =>
          `#${[1, 3, 5]
            .map((i) =>
              Math.round(
                Number.parseInt(a.slice(i, i + 2), 16) * 0.16 +
                  Number.parseInt(b.slice(i, i + 2), 16) * 0.84,
              )
                .toString(16)
                .padStart(2, "0"),
            )
            .join("")}`;
        expect(
          contrast(tokens["--rail-text"], mix(tokens["--system"], tokens["--rail-ground"])),
        ).toBeGreaterThanOrEqual(4.5);
        expect(contrast(tokens["--action-fill"], tokens["--raised"])).toBeGreaterThanOrEqual(3);
      }
    }
  });
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
    "--rule": "#e2e0da",
    "--rule-hard": "#d5d2ca",
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
    "--action-fill": "#2f4858",
    "--on-action-fill": "#ffffff",
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
  it("keeps the page the brightest ground, whatever the grays and climate", () => {
    for (const grays of Object.keys(GRAYS) as GrayKey[]) {
      for (const temperature of TEMPERATURE_KEYS) {
        const light = themeTokens({ ...DEFAULT_THEME, grays, temperature }, "light");
        for (const column of [
          "--rail-ground",
          "--rail-raised",
          "--grail-ground",
          "--sunken",
        ] as const) {
          expect(
            contrast(light["--ground"], light[column]),
            `${column} under ${grays}/${temperature}`,
          ).toBeGreaterThanOrEqual(1);
        }
        const dark = themeTokens({ ...DEFAULT_THEME, grays, temperature }, "dark");
        for (const column of ["--rail-ground", "--grail-ground", "--sunken"] as const) {
          expect(
            contrast(dark["--ground"], dark[column]),
            `${column} under ${grays}/${temperature}`,
          ).toBeGreaterThanOrEqual(1);
        }
      }
    }
  });

  it("moves the page with the temperature rather than painting over it", () => {
    const pages = TEMPERATURE_KEYS.map(
      (temperature) => themeTokens({ ...DEFAULT_THEME, temperature }, "light")["--ground"],
    );
    expect(new Set(pages).size).toBe(TEMPERATURE_KEYS.length);
    for (const temperature of TEMPERATURE_KEYS) {
      const tokens = themeTokens({ ...DEFAULT_THEME, temperature }, "light");
      expect(tokens["--rail-ground"]).not.toBe(tokens["--ground"]);
    }
  });

  it("draws a fill the same on both surfaces, text color included", () => {
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
    const written = themeCss({
      grays: "warm",
      attention: "blue",
      contrast: "more",
      temperature: "warm",
    });
    for (const selector of [PAPER, INK]) {
      expect([...declared(written, selector).keys()]).toEqual([...THEME_TOKENS]);
    }
  });
});

describe("the stylesheet's own values", () => {
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

describe("the grays", () => {
  it("stay under two hundredths of chroma, so no column competes with an agent's color", () => {
    for (const [key, gray] of Object.entries(GRAYS)) {
      expect(gray.c, `${key} grays`).toBeLessThan(0.02);
    }
  });
});
