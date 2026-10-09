import { describe, expect, it, vi } from "vitest";

import {
  BINDINGS,
  type Binding,
  bindingFor,
  type Combo,
  comboLabel,
  formatCombo,
  GLOBAL,
  matches,
  SURFACES,
} from "./keybinds";

/**
 * The key table, under the two questions the app asks it.
 *
 * The table is both a reference the panel draws and a dispatch list the one
 * global handler reads, and only three of its ten rows are the second thing. So
 * most of what is worth testing here is refusal: Escape, Enter, Tab and the
 * arrows are listed for the operator's benefit and must never come back from
 * `bindingFor`, because the global handler calls `preventDefault` on whatever it
 * is handed and would take those keys away from the surfaces that own them.
 *
 * The other half is the rule the table exists to write down once: `mod` is
 * Command or Control on every platform, both accepted. The glyphs are the one
 * thing that does follow the platform, and `IS_MAC` is read once at import, so
 * the glyph test loads its own copies of the module and everything else asserts
 * only what holds either way.
 */

/** A keystroke, the shape the window hands one to the global handler in. */
function press(key: string, held: Partial<KeyboardEventInit> = {}): KeyboardEvent {
  return new KeyboardEvent("keydown", { key, ...held });
}

function row(table: Binding[], id: string): Binding {
  const found = table.find((binding) => binding.id === id);
  if (!found) throw new Error(`no binding named ${id}`);
  return found;
}

/** How the same keystroke would be written down by a keybinding. */
function signature(combo: Combo): string {
  return [combo.mod ? "mod" : "", combo.shift ? "shift" : "", combo.key].join("+");
}

/** A fresh copy of the module, loaded as it would be on the platform named. */
async function loadedOn(platform: string): Promise<typeof import("./keybinds")> {
  vi.resetModules();
  vi.stubGlobal("navigator", { platform, userAgent: platform });
  try {
    return await import("./keybinds");
  } finally {
    vi.unstubAllGlobals();
  }
}

describe("a keystroke that is not the shortcut", () => {
  it("refuses the bare key when the shortcut names a modifier", () => {
    expect(matches(press("k"), { key: "k", mod: true })).toBe(false);
  });

  it("refuses the modified keystroke when the shortcut names no modifier", () => {
    // Both directions have to hold, or Command+K would fire search and whatever
    // plain K is bound to on the same keystroke.
    expect(matches(press("k", { metaKey: true }), { key: "k" })).toBe(false);
    expect(matches(press("k", { ctrlKey: true }), { key: "k" })).toBe(false);
  });

  it("refuses a named key written in the wrong case", () => {
    // Letters are lowercased before comparison and named keys are not, so a row
    // added as `escape` never fires. Better a failing test than a dead listing.
    expect(matches(press("Escape"), { key: "escape" })).toBe(false);
    expect(matches(press("Escape"), { key: "Escape" })).toBe(true);
  });

  it("refuses the plain keystroke when the shortcut names shift", () => {
    expect(matches(press("Enter"), { key: "Enter", shift: true })).toBe(false);
  });

  it("refuses the shifted keystroke for a shortcut that names shift false", () => {
    // The only way to say "this one, and not the shifted one". Naming no shift
    // at all takes both, which is the next case.
    expect(matches(press("Enter", { shiftKey: true }), { key: "Enter", shift: false })).toBe(false);
    expect(matches(press("Enter"), { key: "Enter", shift: false })).toBe(true);
  });
});

describe("a keystroke that is", () => {
  it("takes Command or Control for mod, on either kind of keyboard", () => {
    // The rule the table was written down to state. An operator who learned the
    // shortcut on a laptop should not have to learn it again on a desktop.
    const combo: Combo = { key: "k", mod: true };
    expect(matches(press("k", { metaKey: true }), combo)).toBe(true);
    expect(matches(press("k", { ctrlKey: true }), combo)).toBe(true);
    expect(matches(press("k", { metaKey: true, ctrlKey: true }), combo)).toBe(true);
  });

  it("reads a capital letter as the letter it is", () => {
    // Caps lock or a shifted layout reports "K". A shortcut that quietly stops
    // working under caps lock is indistinguishable from a broken build.
    expect(matches(press("K", { metaKey: true }), { key: "k", mod: true })).toBe(true);
    expect(matches(press("K", { metaKey: true, shiftKey: true }), { key: "k", mod: true })).toBe(
      true,
    );
  });

  it("ignores shift entirely when the shortcut does not mention it", () => {
    expect(matches(press("Enter"), { key: "Enter" })).toBe(true);
    expect(matches(press("Enter", { shiftKey: true }), { key: "Enter" })).toBe(true);
  });
});

describe("which binding a keystroke is", () => {
  it("hands back nothing for the keys their own surface owns", () => {
    // Every one of these is in the table and every one is fixed. If any came
    // back, the global handler would call preventDefault on it: Escape would
    // stop closing what is open and Enter would stop sending the message.
    expect(bindingFor(press("Escape"))).toBeUndefined();
    expect(bindingFor(press("Enter"))).toBeUndefined();
    expect(bindingFor(press("Enter", { shiftKey: true }))).toBeUndefined();
    expect(bindingFor(press("ArrowUp"))).toBeUndefined();
    expect(bindingFor(press("ArrowDown"))).toBeUndefined();
    expect(bindingFor(press("Tab"))).toBeUndefined();
  });

  it("hands back nothing for a letter typed without a modifier", () => {
    expect(bindingFor(press("k"))).toBeUndefined();
    expect(bindingFor(press(","))).toBeUndefined();
    expect(bindingFor(press("/"))).toBeUndefined();
  });

  it("answers with the three that work wherever the operator is", () => {
    expect(bindingFor(press("k", { metaKey: true }))?.id).toBe("search");
    expect(bindingFor(press(",", { metaKey: true }))?.id).toBe("settings");
    expect(bindingFor(press("/", { metaKey: true }))?.id).toBe("shortcuts");
    expect(bindingFor(press("k", { ctrlKey: true }))?.id).toBe("search");
    expect(bindingFor(press(",", { ctrlKey: true }))?.id).toBe("settings");
    expect(bindingFor(press("/", { ctrlKey: true }))?.id).toBe("shortcuts");
  });
});

/**
 * The three global shortcuts, as a literal fixture rather than a read of the
 * table. The key is what the operator reports, the id is what the handler in
 * App.tsx has to switch on, and both are written here so a change to the table
 * makes them fail instead of agreeing with it.
 */
const SHORTCUTS: { what: string; key: string; id: string }[] = [
  { what: "search", key: "k", id: "search" },
  { what: "settings", key: ",", id: "settings" },
  { what: "shortcuts", key: "/", id: "shortcuts" },
];

/**
 * Alt, and every modifier that might arrive alongside it, written out as whole
 * keyboard events.
 *
 * On Windows and Linux AltGr is delivered as Control *and* Alt, so the bare
 * AltGr signal is exactly Control+Alt. The handler calls `preventDefault` on
 * whatever `bindingFor` hands back, so a shortcut that tolerated any of these
 * would swallow the typed character and open a dialog in its place. `what`
 * names the shape so a failure reads as the event that got through, not as an
 * anonymous row of the matrix.
 */
const WITH_ALT: { what: string; held: Partial<KeyboardEventInit> }[] = [
  { what: "Alt", held: { altKey: true } },
  { what: "Shift+Alt", held: { altKey: true, shiftKey: true } },
  { what: "Command+Alt", held: { altKey: true, metaKey: true } },
  { what: "Command+Shift+Alt", held: { altKey: true, metaKey: true, shiftKey: true } },
  { what: "Control+Alt (AltGr)", held: { altKey: true, ctrlKey: true } },
  {
    what: "Control+Shift+Alt (AltGr+Shift)",
    held: { altKey: true, ctrlKey: true, shiftKey: true },
  },
  {
    what: "Command+Control+Alt",
    held: { altKey: true, ctrlKey: true, metaKey: true },
  },
  {
    what: "Command+Control+Shift+Alt",
    held: { altKey: true, ctrlKey: true, metaKey: true, shiftKey: true },
  },
];

/** Neither Alt, nor a bare Shift, is a shortcut: Shift only ever pairs with mod. */
const WITHOUT_A_MODIFIER: { what: string; held: Partial<KeyboardEventInit> }[] = [
  { what: "Shift", held: { shiftKey: true } },
];

/**
 * Command and Control combined with each other and with Shift. `mod` accepts
 * either modifier and both, and Shift is ignored when the binding does not name
 * it, so every one of these is the same shortcut as the bare Command press.
 */
const ACCEPTED: { what: string; held: Partial<KeyboardEventInit> }[] = [
  { what: "Command+Control", held: { metaKey: true, ctrlKey: true } },
  { what: "Command+Shift", held: { metaKey: true, shiftKey: true } },
  { what: "Control+Shift", held: { ctrlKey: true, shiftKey: true } },
  { what: "Command+Control+Shift", held: { metaKey: true, ctrlKey: true, shiftKey: true } },
];

describe("a global shortcut with the wrong modifiers", () => {
  it("refuses every keystroke that arrives with Alt held", () => {
    // AltGr on a Windows or Linux layout is Control+Alt, so these are the events
    // that look most like a shortcut and must produce a character instead.
    for (const { what, key } of SHORTCUTS) {
      for (const { what: held, held: modifiers } of WITH_ALT) {
        expect(bindingFor(press(key, modifiers)), `${held} + ${what}`).toBeUndefined();
      }
    }
  });

  it("refuses Shift on its own, which is how a capital is typed", () => {
    // Shift belongs to mod, never on its own. If it triggered a shortcut, every
    // capital at the start of a sentence would open a dialog.
    for (const { what, key } of SHORTCUTS) {
      for (const { held: modifiers } of WITHOUT_A_MODIFIER) {
        expect(bindingFor(press(key, modifiers)), `Shift + ${what}`).toBeUndefined();
      }
    }
  });

  it("refuses an unknown key however it is modified", () => {
    // A near-miss is the case that matters: `<` and `?` are what a shifted `,`
    // and `/` report on a US layout, and `q` is a plain letter. None of them is
    // in the table, so none may resolve to a shortcut the operator never pressed.
    const unknown: { what: string; held: Partial<KeyboardEventInit> }[] = [
      { what: "q", held: { key: "q" } },
      { what: "F1", held: { key: "F1" } },
      { what: "<", held: { key: "<", shiftKey: true } },
      { what: "?", held: { key: "?", shiftKey: true } },
    ];
    const withModifier: Partial<KeyboardEventInit>[] = [
      { metaKey: true },
      { ctrlKey: true },
      { metaKey: true, ctrlKey: true },
    ];
    for (const { what, held } of unknown) {
      for (const modifier of withModifier) {
        const event = new KeyboardEvent("keydown", { ...held, ...modifier });
        expect(bindingFor(event), `${what} with ${JSON.stringify(modifier)}`).toBeUndefined();
      }
    }
  });
});

describe("a global shortcut with the modifiers it wants", () => {
  it("takes Command, Control, both, and each again with Shift", () => {
    for (const { what, key, id } of SHORTCUTS) {
      for (const { what: held, held: modifiers } of ACCEPTED) {
        expect(bindingFor(press(key, modifiers))?.id, `${held} + ${what}`).toBe(id);
      }
    }
  });

  it("reads an uppercase letter as the letter it is, with either modifier", () => {
    // Caps lock or a shifted layout reports "K". These are the dispatched
    // variants the generic matcher tests do not reach, so a lost lowercasing
    // here is a shortcut that works in lower case and silently not in caps.
    expect(bindingFor(press("K", { ctrlKey: true }))?.id).toBe("search");
    expect(bindingFor(press("K", { ctrlKey: true, shiftKey: true }))?.id).toBe("search");
    expect(bindingFor(press("K", { metaKey: true, ctrlKey: true }))?.id).toBe("search");
    expect(bindingFor(press("K", { metaKey: true, ctrlKey: true, shiftKey: true }))?.id).toBe(
      "search",
    );
    expect(bindingFor(press("K", { shiftKey: true }))).toBeUndefined();
  });
});

describe("the table", () => {
  it("lists every binding once, in a section the panel draws", () => {
    // The panel groups rows by SURFACES, so a `where` outside that list is a
    // shortcut that exists and cannot be found.
    const ids = BINDINGS.map((binding) => binding.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const binding of BINDINGS) {
      expect(SURFACES).toContain(binding.where);
      expect(binding.what.trim()).not.toBe("");
    }
  });

  it("leaves no global shortcut shadowed by an earlier one", () => {
    // `bindingFor` takes the first row that matches, so two non-fixed rows on
    // one combo make the second unreachable and the panel lists a key that
    // does something else.
    const global = BINDINGS.filter((binding) => !binding.fixed);
    const combos = global.map((binding) => signature(binding.combo));
    expect(new Set(combos).size).toBe(combos.length);

    for (const binding of global) {
      const event = press(binding.combo.key, {
        metaKey: Boolean(binding.combo.mod),
        shiftKey: Boolean(binding.combo.shift),
      });
      expect(bindingFor(event)?.id).toBe(binding.id);
    }
  });

  it("offers the one global handler exactly the three ids it dispatches", () => {
    // App.tsx switches on these by name. A fourth non-fixed row is a shortcut
    // the panel promises and nothing answers, and it works everywhere or it is
    // not global, so its section has to say so.
    expect(GLOBAL).toEqual(["search", "settings", "shortcuts"]);
    for (const id of GLOBAL) {
      expect(row(BINDINGS, id).where).toBe("Anywhere");
    }
  });
});

describe("how a shortcut is written down", () => {
  it("writes something legible for every row the panel draws", () => {
    // The label is the whole feature, so no row may render empty, and none may
    // render "undefined": a named key with no glyph falls back to the raw
    // `event.key`, and a row whose combo lost its key would print the word.
    for (const binding of BINDINGS) {
      const label = comboLabel(binding);
      expect(label.trim()).not.toBe("");
      expect(label).not.toContain("undefined");
      expect(formatCombo(binding.combo).trim()).not.toBe("");
      expect(formatCombo(binding.combo)).not.toContain("undefined");
    }
  });

  it("draws the arrow pair as the one shortcut it reads as", () => {
    // Neither arrow takes a modifier, so this holds on both platforms.
    expect(comboLabel(row(BINDINGS, "mention"))).toBe("↑↓");
    expect(comboLabel(row(BINDINGS, "hit"))).toBe("↑↓");
  });

  it("uses the glyphs of the machine it was imported on", async () => {
    // A label naming a key the keyboard does not have is worse than no label.
    // IS_MAC is read once at import, so each platform needs its own copy.
    const mac = await loadedOn("MacIntel");
    expect(mac.formatCombo({ key: "k", mod: true })).toBe("⌘K");
    expect(mac.formatCombo({ key: ",", mod: true })).toBe("⌘,");
    expect(mac.formatCombo({ key: "Enter", shift: true })).toBe("⇧↩");
    expect(mac.comboLabel(row(mac.BINDINGS, "scope"))).toBe("Tab / ⇧Tab");

    const pc = await loadedOn("Win32");
    expect(pc.formatCombo({ key: "k", mod: true })).toBe("Ctrl+K");
    expect(pc.formatCombo({ key: ",", mod: true })).toBe("Ctrl+,");
    expect(pc.formatCombo({ key: "Enter", shift: true })).toBe("Shift+↩");
    expect(pc.comboLabel(row(pc.BINDINGS, "scope"))).toBe("Tab / Shift+Tab");
  });

  it("matches the same keystroke on the platform whose glyphs it does not use", async () => {
    // The glyphs follow the machine and the matching deliberately does not, so
    // a Control-holding operator on a Mac still gets search.
    const mac = await loadedOn("MacIntel");
    expect(mac.bindingFor(press("k", { ctrlKey: true }))?.id).toBe("search");
    expect(mac.bindingFor(press("k", { metaKey: true }))?.id).toBe("search");
  });
});
