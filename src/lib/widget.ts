import atkinsonFace from "@fontsource-variable/atkinson-hyperlegible-next/files/atkinson-hyperlegible-next-latin-wght-normal.woff2?url";
import interFace from "@fontsource-variable/inter/files/inter-latin-opsz-normal.woff2?url";

import type { Surface } from "./appearance";
import type { Typeface } from "./prefs";
import { type Theme, type ThemeToken, themeTokens } from "./theme";

/**
 * A kept page on the status bar: what its condensed view is framed in. No DOM.
 *
 * The frame is on another origin and can load no stylesheet and no font, so
 * nothing about the app reaches it unless it is written into the document the
 * frame is handed. What is written is the bar's own colors as `--guaca-*`,
 * solved from the operator's theme the way the app's are, the bar's own face
 * as an `@font-face` over a `data:` URL, and defaults that set one line of the
 * bar's text on the bar's own ground. The defaults are wrapped in `:where()`,
 * so they have no specificity at all and anything the page says about itself
 * wins.
 *
 * The face is handed over rather than named because the frame's policy allows
 * fonts only as `data:`, and a family named into a document that never loaded
 * it falls through to the system face — which is the one thing a condensed view
 * on the bar must not do, since the bar's own text is beside it.
 */

/** The colors a condensed view is given, by the name the app spells them. */
export const WIDGET_TOKENS = [
  "--text",
  "--muted",
  "--faint",
  "--rule",
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
  typeface: Typeface;
}

export function barLook(
  theme: Theme,
  surface: Surface,
  uiScale: number,
  typeface: Typeface = "inter",
): BarLook {
  const tokens = themeTokens(theme, surface);
  const colors = Object.fromEntries(WIDGET_TOKENS.map((name) => [name, tokens[name]]));
  return {
    surface,
    colors: colors as BarLook["colors"],
    sizePx: (BAR_TYPE_PX * uiScale) / 100,
    typeface,
  };
}

/**
 * The bar's face, as the frame has to be handed it.
 *
 * `family` is the name an `@font-face` declares; `stack` is the fallback chain
 * the frame's text is set in; `src` is the face's own file as a `data:` URL, or
 * null for a face the machine already has. A file that cannot be read leaves
 * `src` null, which falls through the stack instead of leaving the frame
 * undrawn: the wrong face is a smaller failure than no page.
 */
export interface BarFace {
  family: string;
  stack: string;
  src: string | null;
}

const FACES: Record<Typeface, { family: string; stack: string; file: string | null }> = {
  inter: {
    family: '"Inter Variable"',
    stack: '"Inter Variable", system-ui, sans-serif',
    file: interFace,
  },
  system: {
    family: "system-ui",
    stack: 'system-ui, -apple-system, "Segoe UI Variable Text", "Segoe UI", sans-serif',
    file: null,
  },
  legible: {
    family: '"Atkinson Hyperlegible Next Variable"',
    stack: '"Atkinson Hyperlegible Next Variable", system-ui, sans-serif',
    file: atkinsonFace,
  },
};

const faces = new Map<Typeface, Promise<BarFace>>();

/** The bar's face, read once per face and kept. */
export function barFace(typeface: Typeface): Promise<BarFace> {
  const held = faces.get(typeface);
  if (held) return held;
  const { family, stack, file } = FACES[typeface];
  const face =
    file === null
      ? Promise.resolve<BarFace>({ family, stack, src: null })
      : dataUrl(file).then((src) => ({ family, stack, src }));
  faces.set(typeface, face);
  return face;
}

/** A file the app already loads, as a `data:` URL the other origin may load. */
async function dataUrl(file: string): Promise<string | null> {
  try {
    const blob = await (await fetch(file)).blob();
    return await new Promise((ok, bad) => {
      const reader = new FileReader();
      reader.onload = () => ok(String(reader.result));
      reader.onerror = () => bad(reader.error);
      reader.readAsDataURL(blob);
    });
  } catch {
    return null;
  }
}

/** The face with no file yet: what the frame draws in until the file is read. */
function bare(typeface: Typeface): BarFace {
  const { family, stack } = FACES[typeface];
  return { family, stack, src: null };
}

/**
 * The condensed view, with the bar's look written ahead of it.
 *
 * `color-scheme` is the bar's surface because a frame whose scheme differs from
 * the document around it is painted an opaque backdrop by the engine, which on
 * a dark bar is a white box behind a transparent page.
 */
export function dressed(page: string, look: BarLook, face: BarFace = bare(look.typeface)): string {
  const colors = WIDGET_TOKENS.map((name) => `--guaca-${name.slice(2)}: ${look.colors[name]};`);
  const font =
    face.src === null
      ? ""
      : `@font-face { font-family: ${face.family}; font-style: normal; font-display: block; ` +
        `font-weight: 100 900; src: url("${face.src}") format("woff2-variations"); }`;
  return (
    "<style>" +
    font +
    `:root { ${colors.join(" ")} ` +
    `--guaca-font: ${face.stack}; ` +
    `--guaca-size: ${look.sizePx}px; color-scheme: ${look.surface}; }` +
    ":where(html) { background: transparent; color: var(--guaca-text); " +
    "font: var(--guaca-size) / 1.2 var(--guaca-font); }" +
    ":where(html, body) { height: 100%; margin: 0; overflow: hidden; }" +
    ":where(body) { display: flex; align-items: center; gap: 0.4em; white-space: nowrap; }" +
    "</style>" +
    page
  );
}
