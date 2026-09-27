/**
 * A theme, as three things an operator chooses and every color solved from them.
 *
 * What is chosen is how warm or cool the grays are, which hue means "the app
 * wants something from you", and how much contrast the operator wants. No
 * lightness is ever chosen, by the operator or by anybody writing a preset.
 * Each color is solved per surface against a ratio, and the ratio is the
 * design: a combination nobody has looked at clears the same gates the default
 * does, because the gates are where the values came from.
 *
 * Attention is two colors, not one. `--attention` is an ink, legible as text on
 * every ground it is drawn on. `--attention-fill` is a shape, recognizable as
 * the hue it is and carrying its own text color. One value cannot do both: an
 * amber dark enough to be read as words on white is brown, and signals held to
 * the same text ratio share one luminance, which is the one axis red-green
 * colorblindness keeps. So fills are separated by lightness and gated under
 * simulation, and inks are gated on contrast and always travel with a word.
 *
 * `styles.css` holds the default theme's values as its first paint, and
 * `theme.test.ts` holds them equal to what this file derives. Everything else
 * reaches the document through `themeCss`, as a second pair of token blocks
 * written after the stylesheet, so the surface still switches in CSS alone.
 */

import { colorblindDistance, contrast, distance, hex, solve } from "./color";

/** The surface a block is solved for. Mirrors `appearance.ts`, which owns the choice. */
type Surface = "light" | "dark";

export type GrayKey = "neutral" | "warm" | "avocado" | "slate";
export type AttentionKey = "amber" | "orange" | "blue" | "violet";
export type ContrastLevel = "standard" | "more";

export interface Theme {
  grays: GrayKey;
  attention: AttentionKey;
  contrast: ContrastLevel;
}

export const DEFAULT_THEME: Theme = Object.freeze({
  grays: "neutral",
  attention: "amber",
  contrast: "standard",
});

/** A gray family: an OKLCH hue, and how much chroma the columns may carry. */
interface Grays {
  label: string;
  h: number;
  c: number;
}

/**
 * Chroma stays under two hundredths on purpose. An agent's own color is the
 * saturated thing on screen, and a column that is itself a color is a column
 * every agent's color is competing with.
 */
export const GRAYS: Record<GrayKey, Grays> = {
  neutral: { label: "Neutral", h: 260, c: 0.003 },
  warm: { label: "Warm", h: 89, c: 0.009 },
  avocado: { label: "Avocado", h: 145, c: 0.014 },
  slate: { label: "Slate", h: 250, c: 0.013 },
};

/** An attention hue: OKLCH hue and chroma, and the lightness its fill sits at. */
interface Attention {
  label: string;
  h: number;
  c: number;
  fill: number;
}

/**
 * Only hues that clear every gate on both surfaces are offered, and
 * `theme.test.ts` is what says so. Rose and teal were refused. On ink, each
 * fill sits at the lightness of a routine's green dot, 3.7 and 6.4 from it
 * under colorblindness against a floor of 8, and the two are drawn on the
 * same mark to mean "waiting on you" and "fine".
 */
export const ATTENTIONS: Record<AttentionKey, Attention> = {
  amber: { label: "Amber", h: 72, c: 0.17, fill: 0.82 },
  orange: { label: "Orange", h: 50, c: 0.18, fill: 0.72 },
  blue: { label: "Blue", h: 258, c: 0.17, fill: 0.55 },
  violet: { label: "Violet", h: 295, c: 0.18, fill: 0.55 },
};

export const CONTRASTS: readonly ContrastLevel[] = ["standard", "more"];

/**
 * Grounds by OKLCH lightness. Calibrated on the stylesheet as it stood before
 * themes existed, so the warm grays land on the columns Guaca shipped with.
 */
const GROUNDS: Record<Surface, Record<ContrastLevel, Record<string, number>>> = {
  light: {
    standard: {
      ground: 1,
      raised: 1,
      sunken: 0.964,
      edge: 0.907,
      "rail-ground": 0.964,
      "rail-raised": 1,
      "rail-edge": 0.913,
      "rail-wire": 0.887,
      "rail-sunken": 0.928,
      "grail-ground": 0.936,
    },
    more: {
      ground: 1,
      raised: 1,
      sunken: 0.955,
      edge: 0.8,
      "rail-ground": 0.964,
      "rail-raised": 1,
      "rail-edge": 0.8,
      "rail-wire": 0.78,
      "rail-sunken": 0.92,
      "grail-ground": 0.93,
    },
  },
  dark: {
    standard: {
      ground: 0.168,
      raised: 0.204,
      sunken: 0.144,
      edge: 0.281,
      "rail-ground": 0.195,
      "rail-raised": 0.238,
      "rail-edge": 0.285,
      "rail-wire": 0.305,
      "rail-sunken": 0.163,
      "grail-ground": 0.149,
    },
    more: {
      ground: 0.12,
      raised: 0.16,
      sunken: 0.1,
      edge: 0.42,
      "rail-ground": 0.15,
      "rail-raised": 0.2,
      "rail-edge": 0.42,
      "rail-wire": 0.44,
      "rail-sunken": 0.11,
      "grail-ground": 0.1,
    },
  },
};

/**
 * How much of a family's chroma each ground carries. The page on paper carries
 * none, which is what keeps it the only white thing on screen.
 */
const TINT: Record<Surface, Record<string, number>> = {
  light: { ground: 0, raised: 0, "rail-raised": 0 },
  dark: {},
};
const TINT_DEFAULT: Record<string, number> = {
  sunken: 0.6,
  edge: 1.2,
  "rail-edge": 1.2,
  "rail-wire": 1.3,
  "rail-sunken": 1.2,
  "grail-ground": 1.3,
};

/** What each role is solved to. `more` is WCAG AAA for text and muted text. */
export const RATIOS: Record<
  ContrastLevel,
  { text: number; muted: number; faint: number; control: number; signal: number; system: number }
> = {
  standard: { text: 15, muted: 7, faint: 4.5, control: 3, signal: 4.5, system: 7 },
  more: { text: 17, muted: 10, faint: 7, control: 4.5, signal: 7, system: 10 },
};

/** The fixed fill a failure is drawn in, which every attention hue is checked against. */
const DANGER_FILL = { h: 25, c: 0.2, L: 0.52 };

/** Text on a fill: whichever of these two the fill is further from. */
const ON_LIGHT = "#161616";
const ON_DARK = "#ffffff";

function onFill(fill: string): string {
  return contrast(fill, ON_LIGHT) >= contrast(fill, ON_DARK) ? ON_LIGHT : ON_DARK;
}

/** Every token a theme owns, in the order the stylesheet declares them. */
export const THEME_TOKENS = [
  "--rail-ground",
  "--rail-raised",
  "--rail-edge",
  "--rail-text",
  "--rail-muted",
  "--rail-wire",
  "--rail-sunken",
  "--grail-ground",
  "--ground",
  "--raised",
  "--sunken",
  "--edge",
  "--edge-control",
  "--text",
  "--muted",
  "--faint",
  "--attention",
  "--attention-wash",
  "--attention-fill",
  "--on-attention-fill",
  "--system",
  "--system-wash",
  "--ok",
  "--danger",
  "--danger-fill",
  "--on-danger-fill",
] as const;

export type ThemeToken = (typeof THEME_TOKENS)[number];
export type Tokens = Record<ThemeToken, string>;

/** Every color a theme draws with on one surface. */
export function themeTokens(theme: Theme, surface: Surface): Tokens {
  const dark = surface === "dark";
  const ladder = GROUNDS[surface][theme.contrast];
  const ratio = RATIOS[theme.contrast];
  const { h, c } = GRAYS[theme.grays];
  const tint = { ...TINT_DEFAULT, ...TINT[surface] };
  const ground = (name: string) => hex(ladder[name] as number, c * (tint[name] ?? 1), h);

  const rail = {
    "--rail-ground": ground("rail-ground"),
    "--rail-raised": ground("rail-raised"),
    "--rail-edge": ground("rail-edge"),
    "--rail-wire": ground("rail-wire"),
    "--rail-sunken": ground("rail-sunken"),
    "--grail-ground": ground("grail-ground"),
  };
  const page = {
    "--ground": ground("ground"),
    "--raised": ground("raised"),
    "--sunken": ground("sunken"),
    "--edge": ground("edge"),
  };

  // Text is solved against the ground it has least contrast with: the darkest
  // column on paper, the lightest raised surface on ink. Faint is held on the
  // grounds a hint is actually written on, which are not the crews' column.
  const worst = dark
    ? [rail["--rail-raised"], page["--raised"], page["--ground"]]
    : [rail["--grail-ground"], rail["--rail-sunken"], page["--sunken"], page["--ground"]];
  const reading = dark
    ? [rail["--rail-raised"], page["--raised"]]
    : [rail["--rail-ground"], page["--sunken"], page["--ground"]];
  const controls = dark
    ? [page["--ground"], page["--raised"], rail["--rail-ground"], rail["--rail-raised"]]
    : [page["--ground"], rail["--rail-ground"]];

  const text = solve(c * 0.3, h, worst, ratio.text, dark ? 0.97 : 0.2, dark);
  const muted = solve(c, h, worst, ratio.muted, dark ? 0.72 : 0.46, dark);

  const attention = ATTENTIONS[theme.attention];
  const attentionFill = hex(attention.fill, attention.c, attention.h);
  const dangerFill = hex(DANGER_FILL.L, DANGER_FILL.c, DANGER_FILL.h);

  return {
    ...rail,
    "--rail-text": text,
    "--rail-muted": muted,
    ...page,
    "--edge-control": solve(c, h, controls, ratio.control, dark ? 0.4 : 0.72, dark),
    "--text": text,
    "--muted": muted,
    "--faint": solve(c * 1.2, h, reading, ratio.faint, dark ? 0.6 : 0.62, dark),
    "--attention": solve(attention.c, attention.h, worst, ratio.signal, dark ? 0.78 : 0.6, dark),
    "--attention-wash": hex(dark ? 0.26 : 0.965, dark ? 0.045 : 0.035, attention.h),
    "--attention-fill": attentionFill,
    "--on-attention-fill": onFill(attentionFill),
    "--system": solve(0.045, 237, worst, ratio.system, dark ? 0.75 : 0.39, dark),
    "--system-wash": hex(dark ? 0.25 : 0.965, dark ? 0.02 : 0.012, 237),
    "--ok": solve(0.13, 160, worst, ratio.signal, dark ? 0.75 : 0.55, dark),
    "--danger": solve(0.19, 25, worst, ratio.signal, dark ? 0.7 : 0.55, dark),
    "--danger-fill": dangerFill,
    "--on-danger-fill": onFill(dangerFill),
  };
}

/** The theme as two token blocks, one per surface, keyed the way `styles.css` keys its own. */
export function themeCss(theme: Theme): string {
  const block = (selector: string, tokens: Tokens) =>
    `${selector} {\n${THEME_TOKENS.map((name) => `  ${name}: ${tokens[name]};`).join("\n")}\n}`;
  return [
    block(":root", themeTokens(theme, "light")),
    block(':root[data-surface="dark"]', themeTokens(theme, "dark")),
  ].join("\n");
}

export interface Check {
  name: string;
  value: number;
  target: number;
}

/**
 * Every ratio and separation a theme has to clear on one surface, measured.
 * A check passes when `value >= target`; the test suite is what insists.
 */
export function measure(tokens: Tokens, level: ContrastLevel): Check[] {
  const ratio = RATIOS[level];
  const least = (fg: ThemeToken, grounds: ThemeToken[]) =>
    Math.min(...grounds.map((g) => contrast(tokens[fg], tokens[g])));
  const pages: ThemeToken[] = ["--ground", "--rail-ground", "--sunken"];
  return [
    {
      name: "text on every ground",
      value: least("--text", [...pages, "--grail-ground", "--rail-raised"]),
      target: ratio.text,
    },
    {
      name: "muted on every ground",
      value: least("--muted", [...pages, "--rail-raised"]),
      target: ratio.muted,
    },
    { name: "faint where hints are written", value: least("--faint", pages), target: ratio.faint },
    {
      name: "a control's border",
      value: least("--edge-control", ["--ground", "--rail-ground"]),
      target: ratio.control,
    },
    { name: "attention as text", value: least("--attention", pages), target: ratio.signal },
    { name: "ok as text", value: least("--ok", pages), target: ratio.signal },
    { name: "danger as text", value: least("--danger", pages), target: ratio.signal },
    { name: "the system's voice", value: least("--system", pages), target: ratio.system },
    {
      name: "text on the attention fill",
      value: contrast(tokens["--attention-fill"], tokens["--on-attention-fill"]),
      target: 4.5,
    },
    {
      name: "text on the danger fill",
      value: contrast(tokens["--danger-fill"], tokens["--on-danger-fill"]),
      target: 4.5,
    },
    {
      name: "attention fill beside danger fill, colorblind",
      value: colorblindDistance(tokens["--attention-fill"], tokens["--danger-fill"]),
      target: 8,
    },
    {
      name: "attention fill beside danger fill",
      value: distance(tokens["--attention-fill"], tokens["--danger-fill"]),
      target: 15,
    },
    {
      name: "attention fill beside ok, colorblind",
      value: colorblindDistance(tokens["--attention-fill"], tokens["--ok"]),
      target: 8,
    },
    {
      name: "attention fill beside ok",
      value: distance(tokens["--attention-fill"], tokens["--ok"]),
      target: 15,
    },
  ];
}
