import type { Surface } from "./appearance";
import { type Theme, type ThemeToken, themeTokens } from "./theme";

/**
 * A kept page on the status bar: what its condensed view is framed in. No DOM.
 *
 * The frame is on another origin and can load no stylesheet and no font, so
 * nothing about the app reaches it unless it is written into the document the
 * frame is handed. What is written is the bar's own colors as `--guaca-*`,
 * solved from the operator's theme the way the app's are, and defaults that
 * set one line of the bar's text on the bar's own ground. The defaults are
 * wrapped in `:where()`, so they have no specificity at all and anything the
 * page says about itself wins.
 *
 * A system face rather than the app's. The app's interface face is a web font
 * bundled with the app, and the frame's policy allows fonts only as `data:`.
 */

/** The colors a condensed view is given, by the name the app spells them. */
export const WIDGET_TOKENS = [
  "--text",
  "--muted",
  "--faint",
  "--edge",
  "--attention",
  "--ok",
  "--danger",
] as const satisfies readonly ThemeToken[];

/**
 * The bar's type size at scale 100, in pixels: `--type-small`, which the
 * `.statusbar` rule sets. `widget.test.ts` reads both out of the stylesheet, so
 * a change to either is a failing test rather than a strip in another size.
 */
export const BAR_TYPE_PX = 13;

/** Everything about the operator's appearance a condensed view is drawn in. */
export interface BarLook {
  surface: Surface;
  colors: Record<(typeof WIDGET_TOKENS)[number], string>;
  sizePx: number;
}

export function barLook(theme: Theme, surface: Surface, uiScale: number): BarLook {
  const tokens = themeTokens(theme, surface);
  const colors = Object.fromEntries(WIDGET_TOKENS.map((name) => [name, tokens[name]]));
  return {
    surface,
    colors: colors as BarLook["colors"],
    sizePx: (BAR_TYPE_PX * uiScale) / 100,
  };
}

/**
 * The condensed view, with the bar's look written ahead of it.
 *
 * `color-scheme` is the bar's surface because a frame whose scheme differs from
 * the document around it is painted an opaque backdrop by the engine, which on
 * a dark bar is a white box behind a transparent page.
 */
export function dressed(page: string, look: BarLook): string {
  const colors = WIDGET_TOKENS.map((name) => `--guaca-${name.slice(2)}: ${look.colors[name]};`);
  return (
    "<style>" +
    `:root { ${colors.join(" ")} ` +
    `--guaca-font: system-ui, -apple-system, "Segoe UI", sans-serif; ` +
    `--guaca-size: ${look.sizePx}px; color-scheme: ${look.surface}; }` +
    ":where(html) { background: transparent; color: var(--guaca-text); " +
    "font: var(--guaca-size) / 1.2 var(--guaca-font); }" +
    ":where(html, body) { height: 100%; margin: 0; overflow: hidden; }" +
    ":where(body) { display: flex; align-items: center; gap: 0.4em; white-space: nowrap; }" +
    "</style>" +
    page
  );
}
