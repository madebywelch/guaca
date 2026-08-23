/**
 * How large the interface draws, and whether the reading column is paper or ink.
 *
 * Both are one write to the root element, because both are already expressed in
 * `styles.css` as one thing: every size in the stylesheet is a `rem`, so scale
 * is a root font size, and every color the reading column uses is a custom
 * property, so the surface is a token block behind an attribute.
 *
 * The rail is not part of either question. It owns `--rail-*`, it is dark in
 * both surfaces by design, and nothing here names those tokens. What scale does
 * to it is what scale does to everything: it grows.
 *
 * `system` is resolved here rather than in a media query. A media query would
 * mean the dark token block written twice, once for the chosen mode and once
 * inside `prefers-color-scheme`, with no way for CSS to share it; two copies of
 * eighteen colors that must agree is a worse bargain than one listener.
 */

import type { SurfaceMode, UiScale } from "./prefs";

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
 * to do the arithmetic itself: the activity board draws its lanes as SVG
 * coordinates, so it needs a width in pixels rather than in `rem`. If either
 * changes, both have to.
 */
export const ROOT_PX = 16;

/** The surface actually drawn. `system` is not one of these. */
export type Surface = "light" | "dark";

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
 * Puts both onto the document, and reports which surface won.
 *
 * The attribute carries the resolved surface, never `system`: the stylesheet
 * should not have to know that a third choice exists, and a rule keyed on
 * `system` would have to duplicate the one keyed on `dark`.
 */
export function applyAppearance(scale: UiScale, mode: SurfaceMode, dark = prefersDark()): Surface {
  const surface = resolveSurface(mode, dark);
  const root = document.documentElement;

  root.style.setProperty("--ui-scale", `${scale / 100}`);
  root.dataset.surface = surface;
  // So the webview draws its own scrollbars and form controls to match. Nothing
  // in the stylesheet reads this; the engine does.
  root.style.colorScheme = surface;

  return surface;
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
