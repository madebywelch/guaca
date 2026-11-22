import {
  ATTENTIONS,
  type AttentionKey,
  CONTRASTS,
  type ContrastLevel,
  DEFAULT_THEME,
  GRAYS,
  type GrayKey,
  TEMPERATURE_KEYS,
  type Temperature,
  type Theme,
} from "./theme";

export type SurfaceMode = "light" | "dark" | "system";

export const UI_SCALES = [90, 100, 110, 125] as const;

export type UiScale = (typeof UI_SCALES)[number];

export type Cast = "cut" | "drawn";

export const CASTS: readonly Cast[] = ["cut", "drawn"];

export type Typeface = "inter" | "system" | "legible";

export const TYPEFACES: readonly Typeface[] = ["inter", "system", "legible"];

export type Reading = "interface" | "serif";

export const READINGS: readonly Reading[] = ["interface", "serif"];

export const READING_SIZES = [15, 16, 17, 18, 20] as const;

export type ReadingSize = (typeof READING_SIZES)[number];

export interface Prefs extends Theme {
  uiScale: UiScale;
  surface: SurfaceMode;
  typeface: Typeface;
  reading: Reading;
  readingSize: ReadingSize;
  cast: Cast;
  showReasoning: boolean;
  seasonal: boolean;
}

export const DEFAULT_PREFS: Prefs = Object.freeze({
  uiScale: 100,
  surface: "system",
  ...DEFAULT_THEME,
  grays: "neutral",
  temperature: "plain",
  typeface: "inter",
  reading: "interface",
  readingSize: 16,
  cast: "drawn",
  showReasoning: false,
  seasonal: true,
}) as Prefs;

const KEY = "guac.prefs";

const LOOK = 2;

function movedFromOldDefault(prefs: Prefs): Prefs {
  if (prefs.grays !== "slate" || prefs.temperature !== "cool") return prefs;
  return { ...prefs, grays: DEFAULT_PREFS.grays, temperature: DEFAULT_PREFS.temperature };
}

const SURFACES: readonly SurfaceMode[] = ["light", "dark", "system"];

function isScale(value: unknown): value is UiScale {
  return UI_SCALES.some((scale) => scale === value);
}

function isSurface(value: unknown): value is SurfaceMode {
  return SURFACES.some((mode) => mode === value);
}

function isCast(value: unknown): value is Cast {
  return CASTS.some((cast) => cast === value);
}

function oneOf<T extends string | number>(options: readonly T[], value: unknown): value is T {
  return options.some((option) => option === value);
}

const GRAY_KEYS = Object.keys(GRAYS) as GrayKey[];
const ATTENTION_KEYS = Object.keys(ATTENTIONS) as AttentionKey[];

export function readPrefs(raw: unknown): Prefs {
  if (typeof raw !== "object" || raw === null) return DEFAULT_PREFS;

  const stored = raw as Partial<Prefs>;

  const pick = <T extends string | number>(
    options: readonly T[],
    value: unknown,
    fallback: T,
  ): T => (oneOf(options, value) ? value : fallback);

  return {
    uiScale: isScale(stored.uiScale) ? stored.uiScale : DEFAULT_PREFS.uiScale,
    surface: isSurface(stored.surface) ? stored.surface : DEFAULT_PREFS.surface,
    grays: pick(GRAY_KEYS, stored.grays, DEFAULT_PREFS.grays),
    attention: pick(ATTENTION_KEYS, stored.attention, DEFAULT_PREFS.attention),
    contrast: pick<ContrastLevel>(CONTRASTS, stored.contrast, DEFAULT_PREFS.contrast),
    temperature: pick<Temperature>(TEMPERATURE_KEYS, stored.temperature, DEFAULT_PREFS.temperature),
    typeface: pick(TYPEFACES, stored.typeface, DEFAULT_PREFS.typeface),
    reading: pick(READINGS, stored.reading, DEFAULT_PREFS.reading),
    readingSize: pick(READING_SIZES, stored.readingSize, DEFAULT_PREFS.readingSize),
    cast: isCast(stored.cast) ? stored.cast : DEFAULT_PREFS.cast,
    showReasoning:
      typeof stored.showReasoning === "boolean"
        ? stored.showReasoning
        : DEFAULT_PREFS.showReasoning,
    seasonal: typeof stored.seasonal === "boolean" ? stored.seasonal : DEFAULT_PREFS.seasonal,
  };
}

export function loadPrefs(): Prefs {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw === null) return DEFAULT_PREFS;
    const parsed: unknown = JSON.parse(raw);
    const prefs = readPrefs(parsed);
    const marked =
      typeof parsed === "object" &&
      parsed !== null &&
      "look" in parsed &&
      typeof parsed.look === "number" &&
      parsed.look >= LOOK;
    return marked ? prefs : movedFromOldDefault(prefs);
  } catch {
    return DEFAULT_PREFS;
  }
}

export function savePrefs(prefs: Prefs): void {
  try {
    localStorage.setItem(KEY, JSON.stringify({ ...prefs, look: LOOK }));
  } catch {}
}
