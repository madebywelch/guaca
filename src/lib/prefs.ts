/**
 * Preferences the operator sets and the runtime never reads.
 *
 * Everything else the operator can change lives in `config.json` and reaches
 * the webview as `Settings`, because the runtime acts on it: an endpoint, a
 * key, a limit. None of what is here means anything to an agent. How large the
 * interface draws, whether the reading column is paper or ink, what colors and
 * faces it is drawn in, how the agents are drawn, which of four things is worth
 * interrupting you for: the runtime would carry these across IPC only to hand
 * them straight back.
 *
 * So they stay on this side, in `localStorage`, the way the inspector's
 * open-or-closed already does. That is a deliberate exception to "the frontend
 * holds nothing durable" and it is narrow on purpose: nothing in here survives
 * being lost, and none of it is worth a migration.
 *
 * One blob under one key rather than a key each, so a read is one parse and a
 * write is one string. Every field is validated on the way in: this file is a
 * text file an operator can edit, a webview can truncate and an older build can
 * have written, and a preference that cannot be read should cost the default
 * rather than the window.
 */

import {
  ATTENTIONS,
  type AttentionKey,
  CONTRASTS,
  type ContrastLevel,
  DEFAULT_THEME,
  GRAYS,
  type GrayKey,
  type Theme,
} from "./theme";

export type SurfaceMode = "light" | "dark" | "system";

/**
 * What an agent doing something can interrupt you for.
 *
 * Five kinds rather than one switch, because they are not the same question.
 * A permission request is blocking and stays blocking until you answer it. An
 * agent that has stopped is blocking and has no clock on it at all. A routine
 * fires in a channel you were never looking at. A conversation finishing is
 * only interesting if you were waiting for it.
 *
 * The first two are close enough to look like one switch and are not: an
 * operator who turns off permission prompts because a crew asks too often has
 * not said they want to stop hearing that an agent has stopped dead, and one
 * switch would take that decision for them.
 */
export type NotifyKind = "decision" | "approval" | "stuck" | "routine" | "settled" | "failed";

export const NOTIFY_KINDS: readonly NotifyKind[] = [
  "decision",
  "approval",
  "stuck",
  "routine",
  "settled",
  "failed",
];

export interface NotifyPrefs {
  /** The master switch. Off means no kind fires, whatever the kinds say. */
  on: boolean;
  kinds: Record<NotifyKind, boolean>;
}

/**
 * The scales offered, as percentages.
 *
 * 100 is what the app has always drawn, so an existing operator sees no change
 * until they ask for one. It stops at 125 because the rail and the inspector
 * are measured in `rem` and therefore grow too: past that they eat a small
 * window rather than making it legible. Both are capped against the viewport in
 * `styles.css` so the worst case is a narrow reading column, not a lost one.
 */
export const UI_SCALES = [90, 100, 110, 125] as const;

export type UiScale = (typeof UI_SCALES)[number];

/**
 * How the agents are drawn. Two casts of the same creatures: `cut` is paper
 * with an eye that has a pupil and lids, `drawn` is a brush line on twos with
 * brows and, where there is room, a mouth. Nothing about an agent changes with
 * it, which is why it is here and not on the agent: an agent stores which
 * character it is, and this says how every character is drawn.
 */
export type Cast = "cut" | "drawn";

export const CASTS: readonly Cast[] = ["cut", "drawn"];

/**
 * The face the interface is set in. Each is a closed stack in `styles.css`,
 * keyed on `data-typeface`, and each brings its own monospace so a timestamp
 * and the name beside it were drawn to sit together.
 *
 * `system` is whatever the operating system draws its own windows in, which on
 * a Mac is SF Pro and on Windows is Segoe UI. `legible` is Atkinson
 * Hyperlegible, drawn for readers with low vision: its letters are built to be
 * told apart, which is a different job from being pleasant.
 */
export type Typeface = "inter" | "system" | "legible";

export const TYPEFACES: readonly Typeface[] = ["inter", "system", "legible"];

/**
 * The face what agents wrote is set in. `interface` is the typeface above;
 * `serif` is Literata, drawn for reading at length on a screen. Only the
 * transcript and the composer change, because that is the only text anybody
 * reads for minutes at a time.
 */
export type Reading = "interface" | "serif";

export const READINGS: readonly Reading[] = ["interface", "serif"];

/**
 * Sizes for that same text, in pixels at interface scale 100.
 *
 * Separate from the scale because they answer different questions. The scale
 * is how large the whole app draws, rail and controls and all; this is how
 * large the part you read is. Somebody who wants long replies in 18px has not
 * asked for an 18px rail.
 */
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
  notify: NotifyPrefs;
}

/**
 * Frozen, all the way down.
 *
 * `readPrefs` hands this back by identity when there is nothing legible to
 * read, so a caller that wrote a preference into it in place rather than
 * copying would poison the defaults for the life of the process. Every caller
 * copies today; freezing is what keeps that from being something to remember.
 */
export const DEFAULT_PREFS: Prefs = Object.freeze({
  uiScale: 100,
  surface: "light",
  ...DEFAULT_THEME,
  typeface: "inter",
  reading: "interface",
  readingSize: 16,
  cast: "drawn",
  showReasoning: false,
  notify: Object.freeze({
    on: true,
    kinds: Object.freeze({
      decision: true,
      approval: true,
      stuck: true,
      routine: true,
      settled: true,
      failed: true,
    }),
  }),
}) as Prefs;

const KEY = "guac.prefs";

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

/** Whether `value` is one of `options`, narrowed to it. */
function oneOf<T extends string | number>(options: readonly T[], value: unknown): value is T {
  return options.some((option) => option === value);
}

const GRAY_KEYS = Object.keys(GRAYS) as GrayKey[];
const ATTENTION_KEYS = Object.keys(ATTENTIONS) as AttentionKey[];

/**
 * Reads one stored blob, keeping whatever is legible and defaulting the rest.
 *
 * Field by field rather than an object spread: a blob written by an older build
 * is missing fields a spread would leave undefined, and one written by a newer
 * one carries fields this build must not trust. Both are the same case as a
 * hand-edited file, and all three should cost exactly the fields they got wrong.
 */
export function readPrefs(raw: unknown): Prefs {
  if (typeof raw !== "object" || raw === null) return DEFAULT_PREFS;

  const stored = raw as Partial<Prefs>;
  const kinds = { ...DEFAULT_PREFS.notify.kinds };
  const storedKinds = stored.notify?.kinds;
  if (typeof storedKinds === "object" && storedKinds !== null) {
    for (const kind of NOTIFY_KINDS) {
      const value = (storedKinds as Record<string, unknown>)[kind];
      if (typeof value === "boolean") kinds[kind] = value;
    }
  }

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
    typeface: pick(TYPEFACES, stored.typeface, DEFAULT_PREFS.typeface),
    reading: pick(READINGS, stored.reading, DEFAULT_PREFS.reading),
    readingSize: pick(READING_SIZES, stored.readingSize, DEFAULT_PREFS.readingSize),
    cast: isCast(stored.cast) ? stored.cast : DEFAULT_PREFS.cast,
    showReasoning:
      typeof stored.showReasoning === "boolean"
        ? stored.showReasoning
        : DEFAULT_PREFS.showReasoning,
    notify: {
      on: typeof stored.notify?.on === "boolean" ? stored.notify.on : DEFAULT_PREFS.notify.on,
      kinds,
    },
  };
}

/**
 * Loads the stored preferences, or the defaults.
 *
 * Private modes and hardened webviews can refuse storage, and a truncated write
 * leaves JSON that will not parse. A forgotten preference is a much smaller
 * problem than a window that will not draw.
 */
export function loadPrefs(): Prefs {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw === null) return DEFAULT_PREFS;
    return readPrefs(JSON.parse(raw));
  } catch {
    return DEFAULT_PREFS;
  }
}

/** As above: not worth telling the operator about. */
export function savePrefs(prefs: Prefs): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // Nothing to do and nothing to say. The preference holds for this session.
  }
}
