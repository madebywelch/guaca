/**
 * The one preferences blob, read back out of a file the operator can edit.
 *
 * `readPrefs` validates field by field, and the reason is that every route this
 * blob takes into the app is a route it can arrive wrong on: an older build
 * wrote fewer fields, a newer one wrote more, a webview torn down mid-write
 * left half a string, and a curious operator edited it by hand. What is under
 * test here is that each of those costs exactly the fields it got wrong and
 * never the window. A scale that is not one of the offered scales becomes a CSS
 * length no button can turn back off; a flag that was coerced rather than
 * checked is a preference the operator never set.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  CASTS,
  DEFAULT_PREFS,
  loadPrefs,
  type Prefs,
  READING_SIZES,
  READINGS,
  readPrefs,
  savePrefs,
  TYPEFACES,
  UI_SCALES,
} from "./prefs";
import { ATTENTIONS, CONTRASTS, GRAYS, THEME_PRESETS } from "./theme";

/**
 * The key the blob lives under, written out again rather than imported.
 *
 * It is not exported, and it is the whole of the contract with every operator
 * who already has preferences: a rename strands all of them silently. Spelling
 * it here means a rename fails this suite instead.
 */
const KEY = "guac.prefs";

/**
 * The defaults, written out independently of the module's own constant.
 *
 * Neutral grays on a plain page and the system surface are the first look a
 * new operator gets.
 * Asserting against the module constant would pass whatever it said.
 */
const DEFAULTS: Prefs = {
  uiScale: 100,
  surface: "system",
  grays: "neutral",
  attention: "amber",
  contrast: "standard",
  temperature: "plain",
  typeface: "inter",
  reading: "interface",
  readingSize: 16,
  cast: "drawn",
  showReasoning: false,
};

/** Runs `body` against a storage that behaves the way a hardened webview does. */
function withStorage(stub: Partial<Storage>, body: () => void): void {
  const held = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  Object.defineProperty(globalThis, "localStorage", { configurable: true, value: stub });
  try {
    body();
  } finally {
    if (held) Object.defineProperty(globalThis, "localStorage", held);
    else Reflect.deleteProperty(globalThis, "localStorage");
  }
}

beforeEach(() => {
  // One Map backs storage for the whole file, so a leftover blob from the test
  // above is a first launch that is not one.
  localStorage.clear();
});

describe("a blob that cannot be trusted", () => {
  it("retains an existing Ledger preference instead of replacing it with Signal", () => {
    expect(
      readPrefs({
        surface: "light",
        grays: "warm",
        attention: "amber",
        contrast: "standard",
        temperature: "warm",
        reading: "serif",
      }),
    ).toMatchObject({
      surface: "light",
      ...THEME_PRESETS.Paper,
    });
  });

  it("round trips every preset and surface", () => {
    for (const preset of Object.values(THEME_PRESETS)) {
      for (const surface of ["light", "dark", "system"] as const) {
        const prefs = { ...DEFAULTS, ...preset, surface };
        savePrefs(prefs);
        expect(loadPrefs()).toEqual(prefs);
      }
    }
  });
  it("takes the defaults from anything that is not an object", () => {
    // The last two are the ones that actually happen: a blob stringified twice
    // parses back to a string, and a truncated file can parse to a bare number.
    for (const raw of [null, undefined, true, 7, "", '{"uiScale":125}']) {
      expect(readPrefs(raw)).toEqual(DEFAULTS);
    }
  });

  it("takes the defaults from a JSON array, which is an object holding nothing", () => {
    // `typeof [] === "object"`, so an array walks past the guard and into the
    // field reads. Nothing it carries is a field this build ever wrote.
    expect(readPrefs([])).toEqual(DEFAULTS);
    expect(readPrefs([{ uiScale: 125, surface: "dark" }])).toEqual(DEFAULTS);
  });

  it("defaults every field whose type is wrong and keeps the ones that are not", () => {
    // The hand-edited case. The quoted 110 would reach `styles.css` as a
    // length, and a value copied through unchecked puts a string where every
    // reader of the record expects one of a closed set. The notify block is an
    // older build's, and goes whatever it holds.
    expect(
      readPrefs({
        uiScale: "110",
        surface: 1,
        grays: 3,
        attention: ["amber"],
        contrast: true,
        typeface: {},
        reading: null,
        readingSize: "18",
        cast: true,
        notify: { on: false, kinds: { approval: 0, settled: "yes", routine: false } },
      }),
    ).toEqual(DEFAULTS);
  });

  it("refuses a scale that is a number nobody offers", () => {
    // The value drives a CSS length and the preset row the settings dialog
    // highlights, so an unoffered number draws a window with no button lit that
    // would return it to something legible.
    for (const uiScale of [137, 0, 100.5, -100, Number.NaN, 1e9]) {
      expect(readPrefs({ uiScale }).uiScale).toBe(DEFAULTS.uiScale);
    }
  });

  it("fills in what an older build never wrote and keeps what it did", () => {
    expect(readPrefs({ surface: "dark" })).toEqual({ ...DEFAULTS, surface: "dark" });
    // What an older build wrote about notifications is the one thing it wrote
    // that is not kept: those switches are the Host's now.
    expect(readPrefs({ notify: { kinds: { failed: false } } })).toEqual(DEFAULTS);
  });

  it("drops the notify block an older build wrote, whatever shape it is in", () => {
    // Kept, it would be written back on every save forever, as switches no
    // pane draws and nothing reads. The fields beside it survive.
    for (const notify of ["off", [], null, { on: 0, kinds: "all" }, { on: false, kinds: {} }]) {
      expect(readPrefs({ uiScale: 125, notify })).toEqual({ ...DEFAULTS, uiScale: 125 });
    }
    localStorage.setItem(KEY, JSON.stringify({ surface: "dark", notify: { on: false } }));
    savePrefs(loadPrefs());
    expect(JSON.parse(localStorage.getItem(KEY) ?? "{}")).toEqual({
      ...DEFAULTS,
      surface: "dark",
      look: 2,
    });
    expect(DEFAULT_PREFS).toEqual(DEFAULTS);
  });
});

describe("storage that will not cooperate", () => {
  it("reads the defaults when the stored blob will not parse", () => {
    // A write cut short by a webview going away leaves exactly this.
    localStorage.setItem(KEY, '{"uiScale":125,"surface":"da');
    expect(loadPrefs()).toEqual(DEFAULTS);
  });

  it("reads the defaults when storage refuses to be read at all", () => {
    // Private modes and hardened webviews throw on access rather than returning
    // null. A forgotten preference is a far smaller problem than no window.
    withStorage(
      {
        getItem: () => {
          throw new Error("access to storage is not allowed from this context");
        },
      },
      () => {
        expect(loadPrefs()).toEqual(DEFAULTS);
      },
    );
  });

  it("swallows a save that storage refuses", () => {
    // Over quota is the common one. The operator's choice holds for the session
    // either way, and there is nothing for them to do about it.
    const refused = vi.fn(() => {
      throw new Error("exceeded the quota");
    });
    withStorage({ getItem: () => null, setItem: refused }, () => {
      expect(() => savePrefs({ ...DEFAULTS, surface: "dark" })).not.toThrow();
    });
    expect(refused).toHaveBeenCalledOnce();
  });
});

describe("the default look this build replaced", () => {
  /** What every build before this one wrote back, chosen or not. */
  const oldDefault = { ...DEFAULTS, grays: "slate", temperature: "cool" };

  it("does not move a slate that was saved under this build", () => {
    savePrefs({ ...DEFAULTS, ...THEME_PRESETS.Signal!, surface: "dark" });
    expect(loadPrefs()).toMatchObject({ grays: "slate", temperature: "cool", surface: "dark" });
  });

  it("does not move slate grays on a temperature the old default never had", () => {
    localStorage.setItem(KEY, JSON.stringify({ ...oldDefault, temperature: "warm" }));
    expect(loadPrefs()).toMatchObject({ grays: "slate", temperature: "warm" });
  });

  it("moves an unmarked old default to the new one and keeps everything else", () => {
    localStorage.setItem(KEY, JSON.stringify({ ...oldDefault, surface: "dark", uiScale: 125 }));
    expect(loadPrefs()).toEqual({ ...DEFAULTS, surface: "dark", uiScale: 125 });
  });

  it("moves it once: slate picked again after the move stays slate", () => {
    localStorage.setItem(KEY, JSON.stringify(oldDefault));
    const moved = loadPrefs();
    savePrefs({ ...moved, grays: "slate", temperature: "cool" });
    expect(loadPrefs()).toMatchObject({ grays: "slate", temperature: "cool" });
  });

  it("does not move a slate saved by a newer build", () => {
    localStorage.setItem(KEY, JSON.stringify({ ...oldDefault, look: 3 }));
    expect(loadPrefs()).toMatchObject({ grays: "slate", temperature: "cool" });
  });

  it("still moves a blob whose mark is older than this build's", () => {
    localStorage.setItem(KEY, JSON.stringify({ ...oldDefault, look: 1 }));
    expect(loadPrefs()).toEqual(DEFAULTS);
  });
});

describe("a preference that outlives the window", () => {
  it("is the defaults on a first launch, with nothing stored", () => {
    expect(localStorage.getItem(KEY)).toBeNull();
    expect(loadPrefs()).toEqual(DEFAULTS);
  });

  it("reads back exactly what was written", () => {
    const chosen: Prefs = {
      uiScale: 125,
      surface: "dark",
      grays: "avocado",
      attention: "violet",
      contrast: "more",
      temperature: "cool",
      typeface: "legible",
      reading: "serif",
      readingSize: 20,
      cast: "cut",
      showReasoning: true,
    };
    savePrefs(chosen);
    expect(loadPrefs()).toEqual(chosen);
    // One blob under one key: a key per field is a read per field, and a set of
    // them half written by an interrupted save.
    expect(localStorage.length).toBe(1);
  });

  it("keeps every scale the picker offers", () => {
    // `isScale` and `UI_SCALES` are two lists that have to stay one. A scale the
    // dialog offers and the reader rejects is a button that does not hold.
    for (const scale of UI_SCALES) {
      expect(readPrefs({ uiScale: scale }).uiScale).toBe(scale);
    }
  });

  it("keeps every surface, including the one that is not a color", () => {
    // System is the one worth naming: dropped from the reader, it reverts to
    // light, and the operator's window stops following the OS at every launch.
    for (const surface of ["light", "dark", "system"] as const) {
      expect(readPrefs({ surface }).surface).toBe(surface);
    }
  });

  it("keeps every theme and type choice the pane offers, and refuses the rest", () => {
    // Each list the pane draws buttons from and the reader that checks what
    // comes back are two lists that have to stay one. A value the pane offers
    // and the reader drops is a button that does not hold across a restart.
    // The refusals are the ones that actually arrive: a hue this build no
    // longer offers, a reading size that is a scale, a face by its label.
    const offered = {
      grays: Object.keys(GRAYS),
      attention: Object.keys(ATTENTIONS),
      contrast: CONTRASTS,
      typeface: TYPEFACES,
      reading: READINGS,
      readingSize: READING_SIZES,
    } as const;
    const refused = {
      grays: ["Neutral", "gray"],
      attention: ["rose", "teal", "#ffb346"],
      contrast: ["high", "More"],
      typeface: ["Inter", "atkinson"],
      reading: ["literata", "serif "],
      readingSize: [14, 110, 16.5],
    };
    for (const [field, values] of Object.entries(offered)) {
      for (const value of values) {
        expect(readPrefs({ [field]: value })[field as keyof typeof offered]).toBe(value);
      }
      for (const value of refused[field as keyof typeof refused]) {
        expect(readPrefs({ [field]: value })[field as keyof typeof offered]).toBe(
          DEFAULTS[field as keyof typeof offered],
        );
      }
    }
  });

  it("keeps both casts and refuses a name nobody offers", () => {
    // `isCast` and `CASTS` are two lists that have to stay one, and a cast the
    // reader does not know is the default rather than a creature with no
    // drawing: `ink` is the dark surface's name, not a cast's, and the one an
    // operator editing the file by hand is likeliest to type.
    for (const cast of CASTS) {
      expect(readPrefs({ cast }).cast).toBe(cast);
    }
    for (const cast of ["ink", "Drawn", "", null]) {
      expect(readPrefs({ cast }).cast).toBe(DEFAULTS.cast);
    }
  });
});

it("validates the reasoning display preference without coercing old or malformed values", () => {
  expect(readPrefs({ showReasoning: true }).showReasoning).toBe(true);
  for (const showReasoning of [undefined, null, "true", 1, false]) {
    expect(readPrefs({ showReasoning }).showReasoning).toBe(false);
  }
});
