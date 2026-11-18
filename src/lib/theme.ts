import type { Surface } from "./appearance";
import { colorblindDistance, contrast, distance, hex, solve } from "./color";

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

export const TEMPERATURES: Record<Temperature, { label: string; lift: number; tint: number }> = {
  cool: { label: "Cool", lift: 0.018, tint: 0.12 },
  plain: { label: "Plain", lift: 0.014, tint: 0.4 },
  warm: { label: "Warm", lift: 0, tint: 1 },
};

export const TEMPERATURE_KEYS = Object.keys(TEMPERATURES) as Temperature[];

interface Grays {
  label: string;
  h: number;
  c: number;
}

export const GRAYS: Record<GrayKey, Grays> = {
  neutral: { label: "Neutral", h: 260, c: 0.003 },
  warm: { label: "Warm", h: 89, c: 0.009 },
  avocado: { label: "Avocado", h: 165, c: 0.014 },
  slate: { label: "Slate", h: 250, c: 0.013 },
};

interface Attention {
  label: string;
  h: number;
  c: number;
  fill: number;
}

export const ATTENTIONS: Record<AttentionKey, Attention> = {
  amber: { label: "Amber", h: 72, c: 0.17, fill: 0.82 },
  orange: { label: "Orange", h: 50, c: 0.18, fill: 0.72 },
  blue: { label: "Blue", h: 258, c: 0.17, fill: 0.55 },
  violet: { label: "Violet", h: 295, c: 0.18, fill: 0.55 },
};

export const CONTRASTS: readonly ContrastLevel[] = ["standard", "more"];

const GROUNDS: Record<Surface, Record<ContrastLevel, Record<string, number>>> = {
  light: {
    standard: {
      ground: 0.982,
      raised: 0.982,
      sunken: 0.9515,
      popover: 0.982,
      rule: 0.889,
      "rule-hard": 0.833,
      "rail-ground": 0.9577,
      "rail-raised": 0.982,
      "rail-popover": 0.982,
      "rail-edge": 0.889,
      "rail-wire": 0.856,
      "rail-sunken": 0.938,
      "grail-ground": 0.936,
    },
    more: {
      ground: 0.982,
      raised: 0.982,
      sunken: 0.9515,
      popover: 0.982,
      rule: 0.86,
      "rule-hard": 0.79,
      "rail-ground": 0.9577,
      "rail-raised": 0.982,
      "rail-popover": 0.982,
      "rail-edge": 0.86,
      "rail-wire": 0.83,
      "rail-sunken": 0.938,
      "grail-ground": 0.945,
    },
  },
  dark: {
    standard: {
      sunken: 0.182,
      ground: 0.222,
      raised: 0.269,
      popover: 0.313,
      rule: 0.36,
      "rule-hard": 0.42,
      "rail-sunken": 0.182,
      "rail-ground": 0.222,
      "rail-raised": 0.269,
      "rail-popover": 0.313,
      "rail-edge": 0.36,
      "rail-wire": 0.39,
      "grail-ground": 0.182,
    },
    more: {
      sunken: 0.05,
      ground: 0.165,
      raised: 0.219,
      popover: 0.255,
      rule: 0.36,
      "rule-hard": 0.44,
      "rail-sunken": 0.05,
      "rail-ground": 0.165,
      "rail-raised": 0.219,
      "rail-popover": 0.255,
      "rail-edge": 0.36,
      "rail-wire": 0.4,
      "grail-ground": 0.05,
    },
  },
};

const TINT_DEFAULT: Record<string, number> = {
  ground: 0.91,
  raised: 0.91,
  sunken: 1.38,
  popover: 0.91,
  rule: 2.35,
  "rule-hard": 2.86,
  "rail-ground": 1.51,
  "rail-raised": 0.91,
  "rail-popover": 0.91,
  "rail-edge": 2.35,
  "rail-wire": 2.6,
  "rail-sunken": 1.5,
  "grail-ground": 1.6,
};

export const RATIOS: Record<
  ContrastLevel,
  { text: number; muted: number; faint: number; control: number; signal: number; system: number }
> = {
  standard: { text: 15, muted: 7, faint: 4.5, control: 3, signal: 4.5, system: 7 },
  more: { text: 17, muted: 10, faint: 7, control: 4.5, signal: 7, system: 10 },
};

const DANGER_FILL = { h: 25, c: 0.2, L: 0.52 };

const ON_LIGHT = "#161616";
const ON_DARK = "#ffffff";

function onFill(fill: string): string {
  return contrast(fill, ON_LIGHT) >= contrast(fill, ON_DARK) ? ON_LIGHT : ON_DARK;
}

export const THEME_TOKENS = [
  "--rail-ground",
  "--rail-raised",
  "--rail-popover",
  "--rail-edge",
  "--rail-text",
  "--rail-muted",
  "--rail-wire",
  "--rail-sunken",
  "--grail-ground",
  "--ground",
  "--raised",
  "--sunken",
  "--popover",
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
  "--action-fill",
  "--on-action-fill",
  "--ok",
  "--danger",
  "--danger-fill",
  "--on-danger-fill",
] as const;

export type ThemeToken = (typeof THEME_TOKENS)[number];
export type Tokens = Record<ThemeToken, string>;

export function themeTokens(theme: Theme, surface: Surface): Tokens {
  const dark = surface === "dark";
  const base = GROUNDS[surface][theme.contrast];
  const ladder =
    dark || theme.contrast !== "standard"
      ? base
      : theme.grays === "slate"
        ? {
            ...base,
            ground: 0.958,
            raised: 0.982,
            "rail-ground": 0.912,
            "rail-raised": 0.942,
            "rail-popover": 0.942,
          }
        : theme.grays === "avocado"
          ? {
              ...base,
              ground: 0.944,
              raised: 0.982,
              "rail-ground": 0.905,
              "rail-raised": 0.942,
              "rail-popover": 0.942,
            }
          : base;
  const ratio = RATIOS[theme.contrast];
  const { h, c } = GRAYS[theme.grays];
  const climate = TEMPERATURES[theme.temperature];
  const lift = dark ? 0 : climate.lift;
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
    "--rail-popover": ground("rail-popover"),
    "--rail-edge": ground("rail-edge"),
    "--rail-wire": ground("rail-wire"),
    "--rail-sunken": ground("rail-sunken"),
    "--grail-ground": ground("grail-ground"),
  };
  const page = {
    "--ground": ground("ground"),
    "--raised": ground("raised"),
    "--sunken": ground("sunken"),
    "--popover": ground("popover"),
    "--rule": ground("rule"),
    "--rule-hard": ground("rule-hard"),
  };

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
    "--action-fill": text,
    "--on-action-fill": page["--ground"],
    "--ok": solve(0.13, 160, worst, ratio.signal, dark ? 0.75 : 0.55, dark),
    "--danger": solve(0.19, 25, worst, ratio.signal, dark ? 0.7 : 0.55, dark),
    "--danger-fill": dangerFill,
    "--on-danger-fill": onFill(dangerFill),
  };
}

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
