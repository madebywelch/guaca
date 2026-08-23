/**
 * The one preferences blob, read back out of a file the operator can edit.
 *
 * `readPrefs` validates field by field, and the reason is that every route this
 * blob takes into the app is a route it can arrive wrong on: an older build
 * wrote fewer fields, a newer one wrote more, a webview torn down mid-write
 * left half a string, and a curious operator edited it by hand. What is under
 * test here is that each of those costs exactly the fields it got wrong and
 * never the window. A scale that is not one of the offered scales becomes a CSS
 * length no button can turn back off; a notify flag that was coerced rather
 * than checked is either an interruption the operator never switched off or one
 * they asked for and will not get.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  DEFAULT_PREFS,
  loadPrefs,
  NOTIFY_KINDS,
  type Prefs,
  readPrefs,
  savePrefs,
  UI_SCALES,
} from "./prefs";

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
 * 100 and light are what the app has always drawn, and every notification is on
 * because a blob that cannot be read must not be able to switch interruptions
 * off. Asserting against the module constant would pass whatever it said.
 */
const DEFAULTS: Prefs = {
  uiScale: 100,
  surface: "light",
  notify: {
    on: true,
    kinds: { approval: true, routine: true, settled: true, failed: true },
  },
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
    // The hand-edited case. A `Boolean(value)` check would read the 0 as an
    // operator switching the blocking approval prompts off, and the quoted 110
    // would reach `styles.css` as a length; a flag copied through unchecked puts
    // a string where every reader of the record expects a boolean.
    expect(
      readPrefs({
        uiScale: "110",
        surface: 1,
        notify: { on: false, kinds: { approval: 0, settled: "yes", routine: false } },
      }),
    ).toEqual({
      uiScale: 100,
      surface: "light",
      notify: {
        on: false,
        kinds: { approval: true, routine: false, settled: true, failed: true },
      },
    });
  });

  it("defaults the whole notify block when the block itself is not an object", () => {
    // A valid field beside a ruined one survives: the blob is not all or nothing.
    expect(readPrefs({ uiScale: 125, notify: "off" })).toEqual({ ...DEFAULTS, uiScale: 125 });
    expect(readPrefs({ notify: [] })).toEqual(DEFAULTS);
    expect(readPrefs({ notify: null })).toEqual(DEFAULTS);
    // A switch and a kinds record that are both present and both unreadable.
    expect(readPrefs({ notify: { on: 0, kinds: "all" } })).toEqual(DEFAULTS);
  });

  it("refuses a scale that is a number nobody offers", () => {
    // The value drives a CSS length and the preset row the settings dialog
    // highlights, so an unoffered number draws a window with no button lit that
    // would return it to something legible.
    for (const uiScale of [137, 0, 100.5, -100, Number.NaN, 1e9]) {
      expect(readPrefs({ uiScale }).uiScale).toBe(DEFAULTS.uiScale);
    }
  });

  it("ignores a notify kind this build has never heard of", () => {
    // A blob from a newer build carries kinds this one cannot draw a row for.
    // Copied through, they persist on the next write forever, and a stale value
    // under a name a later build reuses is a preference nobody set.
    const prefs = readPrefs({ notify: { kinds: { approval: false, chatter: false } } });
    expect(prefs.notify.kinds.approval).toBe(false);
    expect(Object.keys(prefs.notify.kinds)).toEqual([...NOTIFY_KINDS]);
  });

  it("fills in what an older build never wrote and keeps what it did", () => {
    expect(readPrefs({ surface: "dark" })).toEqual({ ...DEFAULTS, surface: "dark" });
    expect(readPrefs({ notify: { kinds: { failed: false } } })).toEqual({
      ...DEFAULTS,
      notify: { on: true, kinds: { ...DEFAULTS.notify.kinds, failed: false } },
    });
  });

  it("leaves the defaults it fell back on alone for the next read", () => {
    // The kinds record is copied before anything is written into it. Writing
    // into `DEFAULT_PREFS.notify.kinds` instead would work once and then hand
    // every later read in the process the last blob's answer.
    expect(readPrefs({ notify: { kinds: { approval: false } } }).notify.kinds.approval).toBe(false);
    expect(readPrefs({}).notify.kinds.approval).toBe(true);
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

describe("a preference that outlives the window", () => {
  it("is the defaults on a first launch, with nothing stored", () => {
    expect(localStorage.getItem(KEY)).toBeNull();
    expect(loadPrefs()).toEqual(DEFAULTS);
  });

  it("reads back exactly what was written", () => {
    const chosen: Prefs = {
      uiScale: 125,
      surface: "dark",
      notify: {
        on: false,
        kinds: { approval: true, routine: false, settled: false, failed: true },
      },
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
});
