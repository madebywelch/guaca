import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

import { contrast, distance, linear, luminance, oklab } from "./color";
import { DEFAULT_PREFS } from "./prefs";
import {
  ATTENTIONS,
  type AttentionKey,
  CONTRASTS,
  DEFAULT_THEME,
  GRAYS,
  type GrayKey,
  measure,
  SEASONS,
  type Season,
  TEMPERATURE_KEYS,
  THEME_PRESETS,
  THEME_TOKENS,
  type Theme,
  type Tokens,
  takesSeason,
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
      expect(violet["--attention-fill"]).toBe(blue["--attention-fill"]);
    }
  });

  it("keeps the preset surfaces and system voices distinct", () => {
    for (const surface of ["light", "dark"] as const) {
      const presets = Object.values(THEME_PRESETS).map((theme) => themeTokens(theme, surface));
      for (let i = 0; i < presets.length; i++) {
        for (let j = i + 1; j < presets.length; j++) {
          expect(
            distance(presets[i]!["--rail-ground"], presets[j]!["--rail-ground"]),
          ).toBeGreaterThan(1);
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

  it("keeps selected rail text readable over the selected row's ink wash", () => {
    for (const theme of Object.values(THEME_PRESETS)) {
      for (const surface of ["light", "dark"] as const) {
        const tokens = themeTokens(theme, surface);
        const mix = (a: string, b: string) =>
          `#${[1, 3, 5]
            .map((i) =>
              Math.round(
                Number.parseInt(a.slice(i, i + 2), 16) * 0.08 +
                  Number.parseInt(b.slice(i, i + 2), 16) * 0.92,
              )
                .toString(16)
                .padStart(2, "0"),
            )
            .join("")}`;
        expect(
          contrast(tokens["--rail-text"], mix(tokens["--rail-text"], tokens["--rail-ground"])),
        ).toBeGreaterThanOrEqual(4.5);
        expect(contrast(tokens["--action-fill"], tokens["--raised"])).toBeGreaterThanOrEqual(3);
      }
    }
  });
  const SHIPPED: Tokens = {
    "--rail-ground": "#f5f3ee",
    "--rail-raised": "#ffffff",
    "--rail-popover": "#ffffff",
    "--rail-edge": "#e5e2da",
    "--rail-text": "#0b0b0a",
    "--rail-muted": "#54524d",
    "--rail-wire": "#ded9d0",
    "--rail-sunken": "#eae7df",
    "--grail-ground": "#eceae2",
    "--ground": "#ffffff",
    "--raised": "#ffffff",
    "--sunken": "#f4f3f0",
    "--popover": "#ffffff",
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

describe("the primary action", () => {
  it.each(
    [...THEMES, ...Object.values(THEME_PRESETS)].flatMap((theme) =>
      (["light", "dark"] as const).map((surface) => [theme, surface] as const),
    ),
  )("is filled in ink with the ground on it, %o on %s", (theme, surface) => {
    const tokens = themeTokens(theme, surface);
    expect(tokens["--action-fill"]).toBe(tokens["--text"]);
    expect(tokens["--on-action-fill"]).toBe(tokens["--ground"]);
    expect(tokens["--action-fill"]).not.toBe(tokens["--system"]);
  });
});

describe("the layers", () => {
  const LADDERS = [
    ["--sunken", "--ground", "--raised", "--popover"],
    ["--rail-sunken", "--rail-ground", "--rail-raised", "--rail-popover"],
  ] as const;

  it.each(THEMES)("steps sunken, ground, raised and popover apart in dark, %o", (theme) => {
    const tokens = themeTokens(theme, "dark");
    for (const [sunken, ground, raised, popover] of LADDERS) {
      for (const [below, above] of [
        [sunken, ground],
        [ground, raised],
        [raised, popover],
      ] as const) {
        expect(luminance(tokens[above]), `${above} is not above ${below}`).toBeGreaterThan(
          luminance(tokens[below]),
        );
        expect(
          contrast(tokens[below], tokens[above]),
          `${below} to ${above}`,
        ).toBeGreaterThanOrEqual(1.08);
      }
      expect(
        contrast(tokens[sunken], tokens[raised]),
        `${sunken} to ${raised}`,
      ).toBeGreaterThanOrEqual(1.2);
    }
  });

  it.each(THEMES)("keeps light flat, with a popover drawn as raised, %o", (theme) => {
    const tokens = themeTokens(theme, "light");
    expect(tokens["--popover"]).toBe(tokens["--raised"]);
    expect(tokens["--rail-popover"]).toBe(tokens["--rail-raised"]);
  });

  it("keeps the default light grounds where they were", () => {
    const tokens = themeTokens(DEFAULT_THEME, "light");
    expect([tokens["--sunken"], tokens["--ground"], tokens["--raised"]]).toEqual([
      "#f2efe6",
      "#fbf9f3",
      "#fbf9f3",
    ]);
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

describe("a season's palette", () => {
  const DEFAULT_LOOK: Theme = {
    grays: DEFAULT_PREFS.grays,
    attention: DEFAULT_PREFS.attention,
    contrast: DEFAULT_PREFS.contrast,
    temperature: DEFAULT_PREFS.temperature,
  };
  const seasons = Object.keys(SEASONS) as Season[];

  it("passes every contrast gate the solver measures, in light and in dark", () => {
    for (const season of seasons) {
      for (const surface of ["light", "dark"] as const) {
        const tokens = themeTokens(DEFAULT_LOOK, surface, season);
        expect(failures(tokens, DEFAULT_LOOK.contrast)).toEqual([]);
        expect(contrast(tokens["--muted"], tokens["--raised"])).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it("moves the grounds and the system's voice off the everyday palette", () => {
    for (const season of seasons) {
      for (const surface of ["light", "dark"] as const) {
        const everyday = themeTokens(DEFAULT_LOOK, surface);
        const seasonal = themeTokens(DEFAULT_LOOK, surface, season);
        for (const token of ["--ground", "--rail-ground", "--sunken", "--system"] as const) {
          expect(distance(seasonal[token], everyday[token])).toBeGreaterThan(0.5);
        }
      }
    }
  });

  it("leaves amber and red exactly as they are every other day", () => {
    const steady = [
      "--attention",
      "--attention-wash",
      "--attention-fill",
      "--on-attention-fill",
      "--danger",
      "--danger-fill",
      "--on-danger-fill",
    ] as const;
    for (const season of seasons) {
      for (const surface of ["light", "dark"] as const) {
        const everyday = themeTokens(DEFAULT_LOOK, surface);
        const seasonal = themeTokens(DEFAULT_LOOK, surface, season);
        for (const token of steady) expect(seasonal[token]).toBe(everyday[token]);
      }
    }
  });

  it("keeps the action in ink: the text color, with the ground written on it", () => {
    for (const season of seasons) {
      for (const surface of ["light", "dark"] as const) {
        const tokens = themeTokens(DEFAULT_LOOK, surface, season);
        expect(tokens["--action-fill"]).toBe(tokens["--text"]);
        expect(tokens["--on-action-fill"]).toBe(tokens["--ground"]);
      }
    }
  });

  it("applies to the default look only, and leaves a theme the operator chose alone", () => {
    expect(takesSeason(DEFAULT_LOOK)).toBe(true);
    for (const [name, preset] of Object.entries(THEME_PRESETS)) {
      const same = takesSeason(preset);
      expect(same, name).toBe(name === "Violet");
      for (const season of seasons) {
        if (same) continue;
        expect(themeCss(preset, season), name).toBe(themeCss(preset));
      }
    }
    for (const season of seasons) {
      expect(themeCss({ ...DEFAULT_LOOK, contrast: "more" }, season)).toBe(
        themeCss({ ...DEFAULT_LOOK, contrast: "more" }),
      );
      expect(themeCss({ ...DEFAULT_LOOK, attention: "blue" }, season)).toBe(
        themeCss({ ...DEFAULT_LOOK, attention: "blue" }),
      );
    }
  });

  it("writes exactly the everyday blocks when there is no season", () => {
    expect(themeCss(DEFAULT_LOOK, null)).toBe(themeCss(DEFAULT_LOOK));
    expect(themeTokens(DEFAULT_LOOK, "light", null)).toEqual(themeTokens(DEFAULT_LOOK, "light"));
  });
});
