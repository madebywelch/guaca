import atkinsonFace from "@fontsource-variable/atkinson-hyperlegible-next/files/atkinson-hyperlegible-next-latin-wght-normal.woff2?url";
import interFace from "@fontsource-variable/inter/files/inter-latin-opsz-normal.woff2?url";

import type { Surface } from "./appearance";
import type { Typeface } from "./prefs";
import { type Theme, type ThemeToken, themeTokens } from "./theme";

export const WIDGET_TOKENS = [
  "--text",
  "--muted",
  "--faint",
  "--rule",
  "--attention",
  "--ok",
  "--danger",
] as const satisfies readonly ThemeToken[];

export const BAR_TYPE_PX = 13;

export const BAR_HEIGHT_PX = 28;

export interface BarLook {
  surface: Surface;
  colors: Record<(typeof WIDGET_TOKENS)[number], string>;
  sizePx: number;
  heightPx: number;
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
    heightPx: (BAR_HEIGHT_PX * uiScale) / 100,
    typeface,
  };
}

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

function bare(typeface: Typeface): BarFace {
  const { family, stack } = FACES[typeface];
  return { family, stack, src: null };
}

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
    `--guaca-size: ${look.sizePx}px; --guaca-height: ${look.heightPx}px; ` +
    `color-scheme: ${look.surface}; }` +
    ":where(html) { background: transparent; color: var(--guaca-text); " +
    "font-family: var(--guaca-font); font-size: var(--guaca-size); " +
    "line-height: calc(var(--guaca-height) - 1px); }" +
    ":where(html, body) { height: 100%; margin: 0; overflow: hidden; }" +
    ":where(body) { display: block; height: 100%; overflow: hidden; " +
    "text-overflow: ellipsis; white-space: nowrap; }" +
    ":where(body > * + *) { margin-left: 0.4em; }" +
    "</style>" +
    page
  );
}
