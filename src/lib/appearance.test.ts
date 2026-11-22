import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  type Appearance,
  applyAppearance,
  prefersDark,
  ROOT_PX,
  resolveSurface,
  watchSystemSurface,
} from "./appearance";
import { DEFAULT_PREFS, type SurfaceMode, type UiScale } from "./prefs";
import { THEME_TOKENS, themeCss } from "./theme";

function look(uiScale: UiScale, surface: SurfaceMode): Appearance {
  return { ...DEFAULT_PREFS, uiScale, surface };
}

function themeStyle(): HTMLStyleElement | null {
  return document.getElementById("guac-theme") as HTMLStyleElement | null;
}

const REAL_MATCH_MEDIA = globalThis.matchMedia;

function inlineProperties(): string[] {
  const { style } = document.documentElement;
  return Array.from({ length: style.length }, (_, index) => style.item(index));
}

function fakeQuery(matches: boolean) {
  const listeners = new Set<(event: MediaQueryListEvent) => void>();
  return {
    matches,
    listeners,
    addEventListener(type: string, fn: (event: MediaQueryListEvent) => void) {
      if (type === "change") listeners.add(fn);
    },
    removeEventListener(type: string, fn: (event: MediaQueryListEvent) => void) {
      if (type === "change") listeners.delete(fn);
    },
    flip(dark: boolean) {
      for (const fn of [...listeners]) fn({ matches: dark } as MediaQueryListEvent);
    },
  };
}

function stubMatchMedia(query: ReturnType<typeof fakeQuery>): string[] {
  const asked: string[] = [];
  globalThis.matchMedia = ((media: string) => {
    asked.push(media);
    return query as unknown as MediaQueryList;
  }) as typeof globalThis.matchMedia;
  return asked;
}

function withoutMatchMedia(): void {
  delete (globalThis as { matchMedia?: unknown }).matchMedia;
}

beforeEach(() => {
  const root = document.documentElement;
  root.removeAttribute("style");
  root.removeAttribute("data-surface");
  root.removeAttribute("data-typeface");
  root.removeAttribute("data-reading");
  root.removeAttribute("data-season");
  themeStyle()?.remove();
});

afterEach(() => {
  globalThis.matchMedia = REAL_MATCH_MEDIA;
});

describe("a webview with no media queries", () => {
  it("reads a light surface rather than throwing", () => {
    withoutMatchMedia();
    expect(prefersDark()).toBe(false);
  });

  it("hands back a callable no-op, so the caller needs no branch", () => {
    withoutMatchMedia();
    const onChange = vi.fn();
    const stop = watchSystemSurface(onChange);

    expect(stop).toBeTypeOf("function");
    expect(() => stop()).not.toThrow();
    expect(onChange).not.toHaveBeenCalled();
  });

  it("still resolves system, and lands the operator on paper", () => {
    withoutMatchMedia();
    expect(applyAppearance(look(100, "system"))).toBe("light");
    expect(document.documentElement.dataset.surface).toBe("light");
  });
});

describe("resolving a mode", () => {
  it("ignores the OS entirely when the operator named a surface", () => {
    expect(resolveSurface("light", true)).toBe("light");
    expect(resolveSurface("light", false)).toBe("light");
    expect(resolveSurface("dark", false)).toBe("dark");
    expect(resolveSurface("dark", true)).toBe("dark");
  });

  it("follows the OS both ways when the mode is system", () => {
    expect(resolveSurface("system", true)).toBe("dark");
    expect(resolveSurface("system", false)).toBe("light");
  });

  it("asks the OS for the dark-scheme query when no answer is handed in", () => {
    const asked = stubMatchMedia(fakeQuery(true));

    expect(resolveSurface("system")).toBe("dark");
    expect(asked).toEqual(["(prefers-color-scheme: dark)"]);
  });
});

describe("what applying an appearance writes", () => {
  it("writes no color onto the root, so a surface cannot pin the columns", () => {
    applyAppearance({ ...look(125, "dark"), grays: "avocado", attention: "violet" });

    const inline = inlineProperties();
    expect(inline.filter((name) => (THEME_TOKENS as readonly string[]).includes(name))).toEqual([]);
    expect(document.documentElement.getAttribute("style")).not.toContain("--rail");
  });

  it("writes the theme as both surfaces' blocks, after everything else in the head", () => {
    const chosen = { ...look(100, "light"), grays: "slate" as const, attention: "blue" as const };
    applyAppearance(chosen);

    expect(themeStyle()?.textContent).toBe(themeCss(chosen));
    expect(themeStyle()?.textContent).toContain(":root {");
    expect(themeStyle()?.textContent).toContain(':root[data-surface="dark"] {');
    expect(document.head.lastElementChild).toBe(themeStyle());
  });

  it("moves the theme back to the end when something was added after it", () => {
    applyAppearance(look(100, "light"));
    const late = document.createElement("style");
    document.head.append(late);

    applyAppearance(look(100, "dark"));
    expect(document.head.lastElementChild).toBe(themeStyle());
    expect(document.querySelectorAll("#guac-theme")).toHaveLength(1);
    late.remove();
  });

  it("leaves the theme's text alone when the theme did not change", () => {
    applyAppearance(look(100, "light"));
    const text = themeStyle()?.firstChild;

    applyAppearance(look(110, "dark"));
    expect(themeStyle()?.firstChild).toBe(text);
  });

  it("names the typeface and the reading face for the stylesheet to key on", () => {
    applyAppearance({ ...look(100, "light"), typeface: "legible", reading: "serif" });
    expect(document.documentElement.dataset.typeface).toBe("legible");
    expect(document.documentElement.dataset.reading).toBe("serif");

    applyAppearance({ ...look(100, "light"), typeface: "system", reading: "interface" });
    expect(document.documentElement.dataset.typeface).toBe("system");
    expect(document.documentElement.dataset.reading).toBe("interface");
  });

  it("sets the reading size in rem, so the interface scale still multiplies it", () => {
    applyAppearance({ ...look(125, "light"), readingSize: 18 });
    expect(document.documentElement.style.getPropertyValue("--type-read")).toBe("1.125rem");

    applyAppearance({ ...look(100, "light"), readingSize: 16 });
    expect(document.documentElement.style.getPropertyValue("--type-read")).toBe("1rem");
  });

  it("never writes the word system, because no rule is keyed on it", () => {
    expect(applyAppearance(look(100, "system"), true)).toBe("dark");
    expect(document.documentElement.getAttribute("data-surface")).toBe("dark");

    expect(applyAppearance(look(100, "system"), false)).toBe("light");
    expect(document.documentElement.getAttribute("data-surface")).toBe("light");
  });

  it("writes the surface the operator named, whatever the OS thinks", () => {
    applyAppearance(look(100, "dark"), false);
    expect(document.documentElement.dataset.surface).toBe("dark");

    applyAppearance(look(100, "light"), true);
    expect(document.documentElement.dataset.surface).toBe("light");
  });

  it("reports the surface that won, so the caller need not resolve it again", () => {
    expect(applyAppearance(look(90, "light"), true)).toBe("light");
    expect(applyAppearance(look(90, "dark"), false)).toBe("dark");
    expect(applyAppearance(look(90, "system"), true)).toBe("dark");
  });

  it("sets the scale as a unitless multiplier, not a percentage and not a length", () => {
    const cases: [UiScale, string][] = [
      [90, "0.9"],
      [100, "1"],
      [110, "1.1"],
      [125, "1.25"],
    ];

    for (const [scale, expected] of cases) {
      applyAppearance(look(scale, "light"));
      expect(document.documentElement.style.getPropertyValue("--ui-scale")).toBe(expected);
    }
  });

  it("tells the engine which scheme to draw its own controls in", () => {
    applyAppearance(look(100, "system"), true);
    expect(document.documentElement.style.colorScheme).toBe("dark");

    applyAppearance(look(100, "system"), false);
    expect(document.documentElement.style.colorScheme).toBe("light");
  });
});

describe("the anchor the scale multiplies", () => {
  it("is the number the stylesheet multiplies too", () => {
    const css = readFileSync(resolve(__dirname, "../styles.css"), "utf8");
    expect(css).toContain(`font-size: calc(${ROOT_PX}px * var(--ui-scale))`);
  });

  it("is what a rem already resolved to, so scale 100 changes nothing", () => {
    expect(ROOT_PX).toBe(16);
  });
});

describe("watching the OS", () => {
  it("asks for the dark-scheme query, since a typo subscribes to nothing", () => {
    const asked = stubMatchMedia(fakeQuery(false));
    watchSystemSurface(vi.fn());
    expect(asked).toEqual(["(prefers-color-scheme: dark)"]);
  });

  it("reports what the event says changed, not what the query said when it was made", () => {
    const query = fakeQuery(false);
    stubMatchMedia(query);
    const onChange = vi.fn();
    watchSystemSurface(onChange);

    query.flip(true);
    expect(onChange).toHaveBeenLastCalledWith(true);

    query.flip(false);
    expect(onChange).toHaveBeenLastCalledWith(false);
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it("hears nothing more once the returned function is called", () => {
    const query = fakeQuery(false);
    stubMatchMedia(query);
    const onChange = vi.fn();

    watchSystemSurface(onChange)();
    expect(query.listeners.size).toBe(0);

    query.flip(true);
    expect(onChange).not.toHaveBeenCalled();
  });
});

describe("a season on the document", () => {
  it("marks the root and writes the season's palette on the default look", () => {
    applyAppearance({ ...look(100, "light"), season: "thanksgiving" }, false);
    expect(document.documentElement.dataset.season).toBe("thanksgiving");
    expect(themeStyle()?.textContent).toBe(themeCss(DEFAULT_PREFS, "thanksgiving"));
    expect(themeStyle()?.textContent).not.toBe(themeCss(DEFAULT_PREFS));
  });

  it("clears the mark and the palette the day after", () => {
    applyAppearance({ ...look(100, "light"), season: "thanksgiving" }, false);
    applyAppearance({ ...look(100, "light"), season: null }, false);
    expect(document.documentElement.hasAttribute("data-season")).toBe(false);
    expect(themeStyle()?.textContent).toBe(themeCss(DEFAULT_PREFS));
  });

  it("writes the same document as before seasons existed when none is named", () => {
    applyAppearance(look(100, "light"), false);
    expect(document.documentElement.hasAttribute("data-season")).toBe(false);
    expect(themeStyle()?.textContent).toBe(themeCss(DEFAULT_PREFS));
  });

  it("keeps a chosen theme's colors and still marks the root for the decorations", () => {
    const chosen = { ...look(100, "light"), grays: "slate" as const, temperature: "cool" as const };
    applyAppearance({ ...chosen, season: "thanksgiving" }, false);
    expect(document.documentElement.dataset.season).toBe("thanksgiving");
    expect(themeStyle()?.textContent).toBe(themeCss(chosen));
  });
});
