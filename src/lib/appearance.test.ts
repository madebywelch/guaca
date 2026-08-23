import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  applyAppearance,
  prefersDark,
  ROOT_PX,
  resolveSurface,
  watchSystemSurface,
} from "./appearance";
import type { UiScale } from "./prefs";

/**
 * Appearance, as the three things the root element is asked to carry.
 *
 * jsdom resolves no `var()` and does no layout, so nothing here can be asked
 * what color it drew. What it can be asked is what `styles.css` is keyed on:
 * the `data-surface` attribute, the `--ui-scale` multiplier, and the
 * `color-scheme` the engine reads for its own controls. Those three are the
 * whole contract between this file and the stylesheet, and each has a wrong
 * value that still draws a window.
 *
 * Two of them are load-bearing past the obvious. `system` must never reach the
 * attribute, because the stylesheet has one dark block and it is keyed on
 * `dark`: written through verbatim, an operator who asked the OS gets paper in
 * a dark room. And no `--rail-*` name may be written at all, because the rail
 * is dark in both surfaces and a surface that quietly repainted it would look
 * deliberate.
 */

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
    expect(applyAppearance(100, "system")).toBe("light");
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
  it("writes no --rail property, so a surface cannot repaint the rail", () => {
    applyAppearance(125, "dark");

    // The rail is dark in both surfaces and pins its own accents. A `--rail-*`
    // override written from here would look like a design decision, and no
    // color assertion in jsdom could ever catch it.
    expect(inlineProperties().filter((name) => name.startsWith("--rail"))).toEqual([]);
    expect(document.documentElement.getAttribute("style")).not.toContain("--rail");
  });

  it("never writes the word system, because no rule is keyed on it", () => {
    expect(applyAppearance(100, "system", true)).toBe("dark");
    expect(document.documentElement.getAttribute("data-surface")).toBe("dark");

    expect(applyAppearance(100, "system", false)).toBe("light");
    expect(document.documentElement.getAttribute("data-surface")).toBe("light");
  });

  it("writes the surface the operator named, whatever the OS thinks", () => {
    applyAppearance(100, "dark", false);
    expect(document.documentElement.dataset.surface).toBe("dark");

    applyAppearance(100, "light", true);
    expect(document.documentElement.dataset.surface).toBe("light");
  });

  it("reports the surface that won, so the caller need not resolve it again", () => {
    expect(applyAppearance(90, "light", true)).toBe("light");
    expect(applyAppearance(90, "dark", false)).toBe("dark");
    expect(applyAppearance(90, "system", true)).toBe("dark");
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
      applyAppearance(scale, "light");
      expect(document.documentElement.style.getPropertyValue("--ui-scale")).toBe(expected);
    }
  });

  it("tells the engine which scheme to draw its own controls in", () => {
    // Nothing in the stylesheet reads this, so a wrong value shows up only as a
    // white scrollbar down the side of a dark window.
    applyAppearance(100, "system", true);
    expect(document.documentElement.style.colorScheme).toBe("dark");

    applyAppearance(100, "system", false);
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
