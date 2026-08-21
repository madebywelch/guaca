/**
 * Preferences the operator sets and the runtime never reads.
 *
 * Everything else the operator can change lives in `config.json` and reaches
 * the webview as `Settings`, because the runtime acts on it: an endpoint, a
 * key, a limit. None of what is here means anything to an agent. How large the
 * interface draws, whether the reading column is paper or ink, which of four
 * things is worth interrupting you for: the runtime would carry these across
 * IPC only to hand them straight back.
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

export type SurfaceMode = "light" | "dark" | "system";

/**
 * What an agent doing something can interrupt you for.
 *
 * Four kinds rather than one switch, because they are not the same question.
 * A permission request is blocking and stays blocking until you answer it. A
 * routine fires in a channel you were never looking at. A conversation
 * finishing is only interesting if you were waiting for it.
 */
export type NotifyKind = "approval" | "routine" | "settled" | "failed";

export const NOTIFY_KINDS: readonly NotifyKind[] = ["approval", "routine", "settled", "failed"];

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

export interface Prefs {
  uiScale: UiScale;
  surface: SurfaceMode;
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
  notify: Object.freeze({
    on: true,
    kinds: Object.freeze({ approval: true, routine: true, settled: true, failed: true }),
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

  return {
    uiScale: isScale(stored.uiScale) ? stored.uiScale : DEFAULT_PREFS.uiScale,
    surface: isSurface(stored.surface) ? stored.surface : DEFAULT_PREFS.surface,
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
