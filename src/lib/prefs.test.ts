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

const KEY = "guac.prefs";

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
    for (const raw of [null, undefined, true, 7, "", '{"uiScale":125}']) {
      expect(readPrefs(raw)).toEqual(DEFAULTS);
    }
  });

  it("takes the defaults from a JSON array, which is an object holding nothing", () => {
    expect(readPrefs([])).toEqual(DEFAULTS);
    expect(readPrefs([{ uiScale: 125, surface: "dark" }])).toEqual(DEFAULTS);
  });

  it("defaults every field whose type is wrong and keeps the ones that are not", () => {
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
    for (const uiScale of [137, 0, 100.5, -100, Number.NaN, 1e9]) {
      expect(readPrefs({ uiScale }).uiScale).toBe(DEFAULTS.uiScale);
    }
  });

  it("fills in what an older build never wrote and keeps what it did", () => {
    expect(readPrefs({ surface: "dark" })).toEqual({ ...DEFAULTS, surface: "dark" });
    expect(readPrefs({ notify: { kinds: { failed: false } } })).toEqual(DEFAULTS);
  });

  it("drops the notify block an older build wrote, whatever shape it is in", () => {
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
    localStorage.setItem(KEY, '{"uiScale":125,"surface":"da');
    expect(loadPrefs()).toEqual(DEFAULTS);
  });

  it("reads the defaults when storage refuses to be read at all", () => {
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
    expect(localStorage.length).toBe(1);
  });

  it("keeps every scale the picker offers", () => {
    for (const scale of UI_SCALES) {
      expect(readPrefs({ uiScale: scale }).uiScale).toBe(scale);
    }
  });

  it("keeps every surface, including the one that is not a color", () => {
    for (const surface of ["light", "dark", "system"] as const) {
      expect(readPrefs({ surface }).surface).toBe(surface);
    }
  });

  it("keeps every theme and type choice the pane offers, and refuses the rest", () => {
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
