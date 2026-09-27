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

/**
 * Appearance, as the things the root element and the head are asked to carry.
 *
 * jsdom resolves no `var()` and does no layout, so nothing here can be asked
 * what color it drew. What it can be asked is what `styles.css` is keyed on:
 * the `data-surface`, `data-typeface` and `data-reading` attributes, the
 * `--ui-scale` multiplier and the `--type-read` length, the `color-scheme` the
 * engine reads for its own controls, and the theme's token blocks. Those are
 * the whole contract between this file and the stylesheet, and each has a
 * wrong value that still draws a window.
 *
 * Two of them are load-bearing past the obvious. `system` must never reach the
 * attribute, because the stylesheet has one dark block and it is keyed on
 * `dark`: written through verbatim, an operator who asked the OS gets paper in
 * a dark room. And no `--rail-*` name may be written at all: the navigation
 * columns follow the surface through the stylesheet's own dark block, so an
 * inline override from here would pin them to one surface and look deliberate.
 */

/** The appearance a first launch draws, with the two fields most tests turn. */
function look(uiScale: UiScale, surface: SurfaceMode): Appearance {
  return { ...DEFAULT_PREFS, uiScale, surface };
}

/** The element the theme is written into, if it has been. */
function themeStyle(): HTMLStyleElement | null {
  return document.getElementById("guac-theme") as HTMLStyleElement | null;
}

/** The stub `test-setup.ts` installs, so a test that replaces it can put it back. */
const REAL_MATCH_MEDIA = globalThis.matchMedia;

/** Every property left on the root, by name. */
function inlineProperties(): string[] {
  const { style } = document.documentElement;
  return Array.from({ length: style.length }, (_, index) => style.item(index));
}

/**
 * A `MediaQueryList` that reports one answer and remembers who is listening.
 *
 * It only registers a `change` listener, so a watcher subscribed to any other
 * event name hears nothing from `flip` and the test fails rather than passing
 * on a listener that would never fire in a real webview.
 */
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
    /** What the OS changing its mind does. */
    flip(dark: boolean) {
      for (const fn of [...listeners]) fn({ matches: dark } as MediaQueryListEvent);
    },
  };
}

/** Puts one query behind `matchMedia`, and collects what was asked for. */
function stubMatchMedia(query: ReturnType<typeof fakeQuery>): string[] {
  const asked: string[] = [];
  globalThis.matchMedia = ((media: string) => {
    asked.push(media);
    return query as unknown as MediaQueryList;
  }) as typeof globalThis.matchMedia;
  return asked;
}

/** What a webview with no media queries at all looks like. */
function withoutMatchMedia(): void {
  delete (globalThis as { matchMedia?: unknown }).matchMedia;
}

beforeEach(() => {
  const root = document.documentElement;
  root.removeAttribute("style");
  root.removeAttribute("data-surface");
  root.removeAttribute("data-typeface");
  root.removeAttribute("data-reading");
  themeStyle()?.remove();
});

afterEach(() => {
  globalThis.matchMedia = REAL_MATCH_MEDIA;
});

describe("a webview with no media queries", () => {
  it("reads a light surface rather than throwing", () => {
    // This is jsdom, and the reason the call is optional. An unguarded
    // `matchMedia` here is not a wrong color, it is a window that never draws.
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
    // The named modes exist to override the OS. Reading the preference and
    // then deferring to `prefers-color-scheme` anyway is the whole bug.
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
    // A typo in the query string matches nothing, so every operator on system
    // silently gets paper and no assertion on the returned surface would say why.
    const asked = stubMatchMedia(fakeQuery(true));

    expect(resolveSurface("system")).toBe("dark");
    expect(asked).toEqual(["(prefers-color-scheme: dark)"]);
  });
});

describe("what applying an appearance writes", () => {
  it("writes no color onto the root, so a surface cannot pin the columns", () => {
    applyAppearance({ ...look(125, "dark"), grays: "avocado", attention: "violet" });

    // The columns follow the surface through a `data-surface` rule, the
    // stylesheet's own or the theme's. A `--rail-*` value written inline from
    // here would pin them to one surface and look like a design decision, and
    // no color assertion in jsdom could catch it. The theme is not an
    // exception: it is written as blocks, not as properties.
    const inline = inlineProperties();
    expect(inline.filter((name) => (THEME_TOKENS as readonly string[]).includes(name))).toEqual([]);
    expect(document.documentElement.getAttribute("style")).not.toContain("--rail");
  });

  it("writes the theme as both surfaces' blocks, after everything else in the head", () => {
    // After, because the blocks match with the stylesheet's own specificity and
    // order is all that makes them win. Both surfaces, so a surface change is
    // the stylesheet's `data-surface` rule and never a rewrite from here.
    const chosen = { ...look(100, "light"), grays: "slate" as const, attention: "blue" as const };
    applyAppearance(chosen);

    expect(themeStyle()?.textContent).toBe(themeCss(chosen));
    expect(themeStyle()?.textContent).toContain(":root {");
    expect(themeStyle()?.textContent).toContain(':root[data-surface="dark"] {');
    expect(document.head.lastElementChild).toBe(themeStyle());
  });

  it("moves the theme back to the end when something was added after it", () => {
    // Vite adds a lazily loaded chunk's stylesheet to the head when the chunk
    // arrives, which is after the theme was first written.
    applyAppearance(look(100, "light"));
    const late = document.createElement("style");
    document.head.append(late);

    applyAppearance(look(100, "dark"));
    expect(document.head.lastElementChild).toBe(themeStyle());
    expect(document.querySelectorAll("#guac-theme")).toHaveLength(1);
    late.remove();
  });

  it("leaves the theme's text alone when the theme did not change", () => {
    // Every write restyles the whole document, and a surface flip or a scale
    // change re-runs this with the same theme.
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
    // A px here would hold the transcript at one size while every other rem in
    // the window grew with the scale.
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
    // The stylesheet says `calc(16px * var(--ui-scale))`. A percentage or a px
    // there is an invalid length, the declaration is dropped, and the operator
    // gets an interface that ignores the slider.
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
    // Nothing in the stylesheet reads this, so a wrong value shows up only as a
    // white scrollbar down the side of a dark window.
    applyAppearance(look(100, "system"), true);
    expect(document.documentElement.style.colorScheme).toBe("dark");

    applyAppearance(look(100, "system"), false);
    expect(document.documentElement.style.colorScheme).toBe("light");
  });
});

describe("the anchor the scale multiplies", () => {
  it("is the number the stylesheet multiplies too", () => {
    // The anchor exists twice on purpose and is tied together by nothing but
    // this: `styles.css` is where it takes effect, and `ROOT_PX` is the copy the
    // activity board needs because it places its lanes as SVG coordinates and
    // has to do the arithmetic itself. Drift shows up as a board whose columns
    // disagree with the type inside them, which is not a thing anyone would
    // think to look at the stylesheet about.
    const css = readFileSync(resolve(__dirname, "../styles.css"), "utf8");
    expect(css).toContain(`font-size: calc(${ROOT_PX}px * var(--ui-scale))`);
  });

  it("is what a rem already resolved to, so scale 100 changes nothing", () => {
    // Neither `:root` nor Tailwind's preflight had ever set a root font size.
    // Anchoring on the 15px body instead would have shrunk the whole interface
    // by 6.25% for an operator who changed nothing.
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
    // A handler that read the query back instead of the event would report the
    // answer from before the change, so the surface would lag one flip behind.
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
    // The caller rebuilds this on every preference change, so a listener that
    // outlives its teardown means every change adds another one.
    const query = fakeQuery(false);
    stubMatchMedia(query);
    const onChange = vi.fn();

    watchSystemSurface(onChange)();
    expect(query.listeners.size).toBe(0);

    query.flip(true);
    expect(onChange).not.toHaveBeenCalled();
  });
});
