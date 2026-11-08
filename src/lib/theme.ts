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
export type Temperature = "cool" | "plain" | "warm";

export interface Theme {
  grays: GrayKey;
  attention: AttentionKey;
  contrast: ContrastLevel;
  temperature: Temperature;
}

/**
 * The default is Ledger: warm grays on a paper page, and the paper itself
 * warm rather than white.
 *
 * `grays` chooses the hue every gray in the app carries and `temperature`
 * chooses how much of it reaches the page and the two columns. They are
 * separate because they answer different questions: an operator who likes a
 * warm rail may still want to read on white, and the page is the one surface
 * long text sits on for minutes at a time.
 */
export const DEFAULT_THEME: Theme = Object.freeze({
  grays: "warm",
  attention: "amber",
  contrast: "standard",
  temperature: "warm",
});

export const THEME_PRESETS: Record<string, Theme & { reading: "interface" | "serif" }> = {
  Signal: {
    grays: "slate",
    attention: "amber",
    contrast: "standard",
    temperature: "cool",
    reading: "interface",
  },
  Field: {
    grays: "avocado",
    attention: "amber",
    contrast: "standard",
    temperature: "cool",
    reading: "serif",
  },
  Violet: {
    grays: "neutral",
    attention: "amber",
    contrast: "standard",
    temperature: "plain",
    reading: "interface",
  },
  Paper: { ...DEFAULT_THEME, reading: "serif" },
};

export function matchesPreset(
  theme: Theme & { reading: string },
  preset: Theme & { reading: string },
): boolean {
  return (
    theme.grays === preset.grays &&
    theme.attention === preset.attention &&
    theme.contrast === preset.contrast &&
    theme.temperature === preset.temperature &&
    theme.reading === preset.reading
  );
}

/**
 * Surface temperature: how far the page and the columns move along the warmth
 * axis the grays already follow.
 *
 * Not a color, which is why it is here rather than a fourth palette. It is a
 * lightness lift applied to every ground (`warm` is the ladder as drawn, so
 * the lift is zero) and a multiplier on the chroma those grounds carry. Every
 * ink is solved against the ground it lands on afterwards, so a paper that
 * moved is a paper the text was measured against, not one it was painted on
 * top of.
 */
export const TEMPERATURES: Record<
  Temperature,
  { label: string; lift: Record<Surface, number>; tint: number }
> = {
  cool: { label: "Cool", lift: { light: 0.018, dark: -0.009 }, tint: 0.12 },
  plain: { label: "Plain", lift: { light: 0.014, dark: -0.006 }, tint: 0.4 },
  warm: { label: "Warm", lift: { light: 0, dark: 0 }, tint: 1 },
};

export const TEMPERATURE_KEYS = Object.keys(TEMPERATURES) as Temperature[];

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
  avocado: { label: "Avocado", h: 165, c: 0.014 },
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
 * Grounds by OKLCH lightness, on the warm page Ledger draws.
 *
 * Calibrated on the mockup rather than invented: paper lands where `#fbf9f3`
 * is, the rail where `#f4f1e7` is, and the two rules at the separations a
 * printed page uses, 1.3 to 1 for a rule and 1.6 for one that has to be found.
 * The ladder is the `warm` climate; `TEMPERATURES` lifts it for the other two,
 * which is what keeps one table instead of three.
 *
 * `rule` is decoration and `rule-hard` is a boundary. They were one `edge`
 * value at 1.3 to 1 doing both jobs, which meant a page that could not say
 * "this is where the next turn starts" any louder than "this is a hairline".
 */
const GROUNDS: Record<Surface, Record<ContrastLevel, Record<string, number>>> = {
  light: {
    standard: {
      ground: 0.982,
      raised: 0.982,
      sunken: 0.9515,
      rule: 0.889,
      "rule-hard": 0.833,
      "rail-ground": 0.9577,
      "rail-raised": 0.982,
      "rail-edge": 0.889,
      "rail-wire": 0.856,
      "rail-sunken": 0.938,
      "grail-ground": 0.936,
    },
    more: {
      ground: 0.982,
      raised: 0.982,
      sunken: 0.9515,
      rule: 0.86,
      "rule-hard": 0.79,
      "rail-ground": 0.9577,
      "rail-raised": 0.982,
      "rail-edge": 0.86,
      "rail-wire": 0.83,
      "rail-sunken": 0.938,
      "grail-ground": 0.945,
    },
  },
  dark: {
    standard: {
      ground: 0.177,
      raised: 0.2,
      sunken: 0.156,
      rule: 0.274,
      "rule-hard": 0.331,
      "rail-ground": 0.202,
      "rail-raised": 0.228,
      "rail-edge": 0.281,
      "rail-wire": 0.3,
      "rail-sunken": 0.168,
      "grail-ground": 0.155,
    },
    more: {
      ground: 0.14,
      raised: 0.165,
      sunken: 0.12,
      rule: 0.33,
      "rule-hard": 0.4,
      "rail-ground": 0.16,
      "rail-raised": 0.19,
      "rail-edge": 0.33,
      "rail-wire": 0.36,
      "rail-sunken": 0.13,
      "grail-ground": 0.115,
    },
  },
};

/**
 * How much of a family's chroma each ground carries, as a multiple of the
 * family's own.
 *
 * The page carries almost all of it now, because the page is paper and paper
 * has a temperature. It carried none while the page was white, which is what
 * made white the only white thing on screen; the warmth arrives through
 * `TEMPERATURES` instead, and an operator who picks `cool` gets a page with a
 * tenth of it. The rules carry the most of all: a hairline one and a third to
 * one against the page reads as either a pencil line or a warm shadow, and the
 * hue is the whole of what makes it the second.
 */
const TINT_DEFAULT: Record<string, number> = {
  ground: 0.91,
  raised: 0.91,
  sunken: 1.38,
  rule: 2.35,
  "rule-hard": 2.86,
  "rail-ground": 1.51,
  "rail-raised": 0.91,
  "rail-edge": 2.35,
  "rail-wire": 2.6,
  "rail-sunken": 1.5,
  "grail-ground": 1.6,
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
  "--rule",
  "--rule-hard",
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
  "--action-fill",
  "--on-action-fill",
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
  const base = GROUNDS[surface][theme.contrast];
  const ladder =
    theme.grays === "slate" && theme.contrast === "standard"
      ? dark
        ? { ...base, ground: 0.229, raised: 0.269, "rail-ground": 0.199, "rail-raised": 0.249 }
        : { ...base, ground: 0.958, raised: 0.982, "rail-ground": 0.912, "rail-raised": 0.942 }
      : theme.grays === "avocado" && theme.contrast === "standard"
        ? dark
          ? { ...base, ground: 0.229, raised: 0.269, "rail-ground": 0.205, "rail-raised": 0.249 }
          : { ...base, ground: 0.944, raised: 0.982, "rail-ground": 0.905, "rail-raised": 0.942 }
        : base;
  const ratio = RATIOS[theme.contrast];
  const { h, c } = GRAYS[theme.grays];
  const climate = TEMPERATURES[theme.temperature];
  const lift = climate.lift[surface];
  const ground = (name: string) =>
    hex(
      Math.min(1, Math.max(dark ? 0.03 : 0, (ladder[name] as number) + lift)),
      c *
        (TINT_DEFAULT[name] ?? 1) *
        (theme.grays === "slate" || theme.grays === "avocado" ? 1 : climate.tint),
      h,
    );

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
    "--rule": ground("rule"),
    "--rule-hard": ground("rule-hard"),
  };

  // Text is solved against the ground it has least contrast with: the darkest
  // column on paper, the lightest raised surface on ink. Faint is held on the
  // grounds a hint is actually written on, which are not the crews' column.
  const worst = dark
    ? [rail["--rail-raised"], page["--raised"], page["--ground"]]
    : [
        rail["--grail-ground"],
        rail["--rail-ground"],
        rail["--rail-sunken"],
        page["--sunken"],
        page["--ground"],
      ];
  const reading = dark
    ? [rail["--rail-raised"], page["--raised"]]
    : [rail["--rail-ground"], page["--sunken"], page["--ground"]];
  const controls = dark
    ? [page["--ground"], page["--raised"], rail["--rail-ground"], rail["--rail-raised"]]
    : [page["--ground"], page["--raised"], rail["--rail-ground"]];

  // Ink carries more of the family's chroma than it used to. A soft black on
  // warm paper is the page's own temperature at the other end of the scale,
  // and a neutral #131314 beside a #fbf9f3 page is the one gray the eye reads
  // as belonging to a different document.
  const text = solve(c * 1.3, h, worst, ratio.text, dark ? 0.97 : 0.2, dark);
  const muted = solve(c * 2.4, h, worst, ratio.muted, dark ? 0.72 : 0.46, dark);

  const attention = ATTENTIONS[theme.attention];
  const attentionFill = hex(attention.fill, attention.c, attention.h);
  const dangerFill = hex(DANGER_FILL.L, DANGER_FILL.c, DANGER_FILL.h);
  const systemHue =
    theme.grays === "neutral"
      ? 295
      : theme.grays === "avocado"
        ? 210
        : theme.grays === "slate"
          ? 255
          : 237;
  const systemChroma = theme.grays === "warm" ? 0.045 : 0.16;
  const system = solve(systemChroma, systemHue, worst, ratio.system, dark ? 0.75 : 0.39, dark);
  const actionFill = theme.grays === "warm" ? text : system;

  return {
    ...rail,
    "--rail-text": text,
    "--rail-muted": muted,
    ...page,
    "--edge-control": solve(c, h, controls, ratio.control, dark ? 0.4 : 0.72, dark),
    "--text": text,
    "--muted": muted,
    "--faint": solve(c * 2.8, h, reading, ratio.faint, dark ? 0.6 : 0.62, dark),
    "--attention": solve(attention.c, attention.h, worst, ratio.signal, dark ? 0.772 : 0.494, dark),
    "--attention-wash": hex(dark ? 0.26 : 0.965, dark ? 0.045 : 0.035, attention.h),
    "--attention-fill": attentionFill,
    "--on-attention-fill": onFill(attentionFill),
    "--system": system,
    "--system-wash": hex(dark ? 0.25 : 0.965, dark ? 0.02 : 0.012, systemHue),
    "--action-fill": actionFill,
    "--on-action-fill": onFill(actionFill),
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
      name: "action fill beside raised",
      value: contrast(tokens["--action-fill"], tokens["--raised"]),
      target: 3,
    },
    {
      name: "text on the action fill",
      value: contrast(tokens["--action-fill"], tokens["--on-action-fill"]),
      target: 4.5,
    },
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
