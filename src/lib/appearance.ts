/**
 * How large the interface draws, whether it is paper or ink, and what colors
 * and faces it is drawn in.
 *
 * Each is one write to the root element, because each is already expressed in
 * `styles.css` as one thing: every size in the stylesheet is a `rem`, so scale
 * is a root font size; every color is a custom property, so the surface is a
 * token block behind an attribute; and every face is a stack behind another.
 *
 * The theme is the one write that is not an attribute. Its colors are solved
 * from what the operator chose, so they cannot be written into the stylesheet
 * ahead of time: `lib/theme.ts` writes them as a second pair of token blocks,
 * keyed exactly as the stylesheet keys its own, in one `<style>` after it.
 * Written that way rather than as properties on the root, the surface still
 * switches through the stylesheet's own `data-surface` rule. A `--rail-*` value
 * written inline would pin the columns to whichever surface was current when it
 * was written, and look like a design decision.
 *
 * `system` is resolved here rather than in a media query. A media query would
 * mean the dark token block written twice, once for the chosen mode and once
 * inside `prefers-color-scheme`, with no way for CSS to share it; two copies of
 * eighteen colors that must agree is a worse bargain than one listener.
 */

import type { Prefs, SurfaceMode } from "./prefs";
import { themeCss } from "./theme";

/**
 * What `1rem` resolves to before scaling.
 *
 * Neither `:root` nor Tailwind's preflight had ever set a root font size, so
 * 16px is what every `rem` in the stylesheet is already measured against;
 * anchoring on the 15px `body` instead would have shrunk the whole interface at
 * scale 100.
 *
 * The same number appears in `styles.css` as `calc(16px * var(--ui-scale))`,
 * which is where it takes effect. This copy exists for the one thing that has
 * to do the arithmetic itself: the flow board draws its lanes as SVG
 * coordinates, so it needs a width in pixels rather than in `rem`. If either
 * changes, both have to.
 */
export const ROOT_PX = 16;

/** The surface actually drawn. `system` is not one of these. */
export type Surface = "light" | "dark";

/** Everything about how the window draws, as the preferences hold it. */
export type Appearance = Pick<
  Prefs,
  | "uiScale"
  | "surface"
  | "grays"
  | "attention"
  | "contrast"
  | "typeface"
  | "reading"
  | "readingSize"
>;

/** The one element the theme's token blocks are written into. */
const THEME_STYLE = "guac-theme";

/** True when the OS has asked for a dark interface. */
export function prefersDark(): boolean {
  // Optional call: jsdom ships no media queries at all, and a preference that
  // cannot be read is the light default rather than a thrown render.
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

export function resolveSurface(mode: SurfaceMode, dark = prefersDark()): Surface {
  if (mode === "system") return dark ? "dark" : "light";
  return mode;
}

/**
 * Puts all of it onto the document, and reports which surface won.
 *
 * The attribute carries the resolved surface, never `system`: the stylesheet
 * should not have to know that a third choice exists, and a rule keyed on
 * `system` would have to duplicate the one keyed on `dark`.
 */
export function applyAppearance(look: Appearance, dark = prefersDark()): Surface {
  const surface = resolveSurface(look.surface, dark);
  const root = document.documentElement;

  root.style.setProperty("--ui-scale", `${look.uiScale / 100}`);
  root.style.setProperty("--type-read", `${look.readingSize / ROOT_PX}rem`);
  root.dataset.surface = surface;
  root.dataset.typeface = look.typeface;
  root.dataset.reading = look.reading;
  // So the webview draws its own scrollbars and form controls to match. Nothing
  // in the stylesheet reads this; the engine does.
  root.style.colorScheme = surface;

  writeTheme(themeCss(look));
  return surface;
}

/**
 * The theme's token blocks, in a `<style>` that comes after the stylesheet.
 *
 * Last in the head, and moved back there if something was appended after it:
 * both blocks match with the same specificity as the stylesheet's own, so order
 * is the whole of what makes them win. Rewritten only when the text differs,
 * because every write restyles the document and this runs on every change of
 * surface too.
 */
function writeTheme(css: string): void {
  const head = document.head;
  let style = document.getElementById(THEME_STYLE);
  if (!(style instanceof HTMLStyleElement)) {
    style = document.createElement("style");
    style.id = THEME_STYLE;
  }
  if (style.textContent !== css) style.textContent = css;
  if (head.lastElementChild !== style) head.append(style);
}

/**
 * Calls back when the OS changes its mind, for as long as the returned function
 * is not called.
 *
 * Only `system` cares, but subscribing unconditionally keeps the caller from
 * having to tear down and rebuild a listener every time the mode changes. A
 * webview with no media queries subscribes to nothing and returns a no-op, so
 * the caller needs no branch either.
 */
export function watchSystemSurface(onChange: (dark: boolean) => void): () => void {
  const query = window.matchMedia?.("(prefers-color-scheme: dark)");
  if (!query) return () => {};

  const handle = (event: MediaQueryListEvent) => onChange(event.matches);
  query.addEventListener("change", handle);
  return () => query.removeEventListener("change", handle);
}
