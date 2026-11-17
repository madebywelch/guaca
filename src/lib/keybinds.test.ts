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

function press(key: string, held: Partial<KeyboardEventInit> = {}): KeyboardEvent {
  return new KeyboardEvent("keydown", { key, ...held });
}

function row(table: Binding[], id: string): Binding {
  const found = table.find((binding) => binding.id === id);
  if (!found) throw new Error(`no binding named ${id}`);
  return found;
}

function signature(combo: Combo): string {
  return [combo.mod ? "mod" : "", combo.shift ? "shift" : "", combo.key].join("+");
}

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
    expect(matches(press("k", { metaKey: true }), { key: "k" })).toBe(false);
    expect(matches(press("k", { ctrlKey: true }), { key: "k" })).toBe(false);
  });

  it("refuses a named key written in the wrong case", () => {
    expect(matches(press("Escape"), { key: "escape" })).toBe(false);
    expect(matches(press("Escape"), { key: "Escape" })).toBe(true);
  });

  it("refuses the plain keystroke when the shortcut names shift", () => {
    expect(matches(press("Enter"), { key: "Enter", shift: true })).toBe(false);
  });

  it("refuses the shifted keystroke for a shortcut that names shift false", () => {
    expect(matches(press("Enter", { shiftKey: true }), { key: "Enter", shift: false })).toBe(false);
    expect(matches(press("Enter"), { key: "Enter", shift: false })).toBe(true);
  });
});

describe("a keystroke that is", () => {
  it("takes Command or Control for mod, on either kind of keyboard", () => {
    const combo: Combo = { key: "k", mod: true };
    expect(matches(press("k", { metaKey: true }), combo)).toBe(true);
    expect(matches(press("k", { ctrlKey: true }), combo)).toBe(true);
    expect(matches(press("k", { metaKey: true, ctrlKey: true }), combo)).toBe(true);
  });

  it("reads a capital letter as the letter it is", () => {
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

  it("answers with the four that work wherever the operator is", () => {
    expect(bindingFor(press("k", { metaKey: true }))?.id).toBe("field");
    expect(bindingFor(press("f", { metaKey: true }))?.id).toBe("search");
    expect(bindingFor(press(",", { metaKey: true }))?.id).toBe("settings");
    expect(bindingFor(press("/", { metaKey: true }))?.id).toBe("shortcuts");
    expect(bindingFor(press("k", { ctrlKey: true }))?.id).toBe("field");
    expect(bindingFor(press("f", { ctrlKey: true }))?.id).toBe("search");
    expect(bindingFor(press(",", { ctrlKey: true }))?.id).toBe("settings");
    expect(bindingFor(press("/", { ctrlKey: true }))?.id).toBe("shortcuts");
  });
});

const SHORTCUTS: { what: string; key: string; id: string }[] = [
  { what: "field", key: "k", id: "field" },
  { what: "search", key: "f", id: "search" },
  { what: "settings", key: ",", id: "settings" },
  { what: "shortcuts", key: "/", id: "shortcuts" },
];

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

const WITHOUT_A_MODIFIER: { what: string; held: Partial<KeyboardEventInit> }[] = [
  { what: "Shift", held: { shiftKey: true } },
];

const ACCEPTED: { what: string; held: Partial<KeyboardEventInit> }[] = [
  { what: "Command+Control", held: { metaKey: true, ctrlKey: true } },
  { what: "Command+Shift", held: { metaKey: true, shiftKey: true } },
  { what: "Control+Shift", held: { ctrlKey: true, shiftKey: true } },
  { what: "Command+Control+Shift", held: { metaKey: true, ctrlKey: true, shiftKey: true } },
];

describe("a global shortcut with the wrong modifiers", () => {
  it("refuses every keystroke that arrives with Alt held", () => {
    for (const { what, key } of SHORTCUTS) {
      for (const { what: held, held: modifiers } of WITH_ALT) {
        expect(bindingFor(press(key, modifiers)), `${held} + ${what}`).toBeUndefined();
      }
    }
  });

  it("refuses Shift on its own, which is how a capital is typed", () => {
    for (const { what, key } of SHORTCUTS) {
      for (const { held: modifiers } of WITHOUT_A_MODIFIER) {
        expect(bindingFor(press(key, modifiers)), `Shift + ${what}`).toBeUndefined();
      }
    }
  });

  it("refuses an unknown key however it is modified", () => {
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
    expect(bindingFor(press("K", { ctrlKey: true }))?.id).toBe("field");
    expect(bindingFor(press("K", { ctrlKey: true, shiftKey: true }))?.id).toBe("field");
    expect(bindingFor(press("K", { metaKey: true, ctrlKey: true }))?.id).toBe("field");
    expect(bindingFor(press("K", { metaKey: true, ctrlKey: true, shiftKey: true }))?.id).toBe(
      "field",
    );
    expect(bindingFor(press("K", { shiftKey: true }))).toBeUndefined();
    expect(bindingFor(press("F", { ctrlKey: true }))?.id).toBe("search");
    expect(bindingFor(press("F", { ctrlKey: true, shiftKey: true }))?.id).toBe("search");
    expect(bindingFor(press("F", { metaKey: true, ctrlKey: true }))?.id).toBe("search");
    expect(bindingFor(press("F", { metaKey: true, ctrlKey: true, shiftKey: true }))?.id).toBe(
      "search",
    );
    expect(bindingFor(press("F", { shiftKey: true }))).toBeUndefined();
  });
});

describe("the table", () => {
  it("lists every binding once, in a section the panel draws", () => {
    const ids = BINDINGS.map((binding) => binding.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const binding of BINDINGS) {
      expect(SURFACES).toContain(binding.where);
      expect(binding.what.trim()).not.toBe("");
    }
  });

  it("leaves no global shortcut shadowed by an earlier one", () => {
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

  it("offers the one global handler exactly the four ids it dispatches", () => {
    expect(GLOBAL).toEqual(["field", "search", "settings", "shortcuts"]);
    for (const id of GLOBAL) {
      expect(row(BINDINGS, id).where).toBe("Anywhere");
    }
  });
});

describe("how a shortcut is written down", () => {
  it("writes something legible for every row the panel draws", () => {
    for (const binding of BINDINGS) {
      const label = comboLabel(binding);
      expect(label.trim()).not.toBe("");
      expect(label).not.toContain("undefined");
      expect(formatCombo(binding.combo).trim()).not.toBe("");
      expect(formatCombo(binding.combo)).not.toContain("undefined");
    }
  });

  it("draws the arrow pair as the one shortcut it reads as", () => {
    expect(comboLabel(row(BINDINGS, "mention"))).toBe("↑↓");
    expect(comboLabel(row(BINDINGS, "hit"))).toBe("↑↓");
  });

  it("uses the glyphs of the machine it was imported on", async () => {
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
    const mac = await loadedOn("MacIntel");
    expect(mac.bindingFor(press("k", { ctrlKey: true }))?.id).toBe("field");
    expect(mac.bindingFor(press("k", { metaKey: true }))?.id).toBe("field");
    expect(mac.bindingFor(press("f", { ctrlKey: true }))?.id).toBe("search");
    expect(mac.bindingFor(press("f", { metaKey: true }))?.id).toBe("search");
  });
});
