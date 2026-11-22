import type { Prefs, SurfaceMode } from "./prefs";
import { type Season, themeCss } from "./theme";

export const ROOT_PX = 16;

export type Surface = "light" | "dark";

export type Appearance = Pick<
  Prefs,
  | "uiScale"
  | "surface"
  | "grays"
  | "attention"
  | "contrast"
  | "temperature"
  | "typeface"
  | "reading"
  | "readingSize"
> & { season?: Season | null };

const THEME_STYLE = "guac-theme";

export function prefersDark(): boolean {
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

export function resolveSurface(mode: SurfaceMode, dark = prefersDark()): Surface {
  if (mode === "system") return dark ? "dark" : "light";
  return mode;
}

export function applyAppearance(look: Appearance, dark = prefersDark()): Surface {
  const surface = resolveSurface(look.surface, dark);
  const root = document.documentElement;

  root.style.setProperty("--ui-scale", `${look.uiScale / 100}`);
  root.style.setProperty("--type-read", `${look.readingSize / ROOT_PX}rem`);
  root.dataset.surface = surface;
  root.dataset.typeface = look.typeface;
  root.dataset.reading = look.reading;
  root.style.colorScheme = surface;
  if (look.season) root.dataset.season = look.season;
  else delete root.dataset.season;

  writeTheme(themeCss(look, look.season ?? null));
  return surface;
}

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

export function watchSystemSurface(onChange: (dark: boolean) => void): () => void {
  const query = window.matchMedia?.("(prefers-color-scheme: dark)");
  if (!query) return () => {};

  const handle = (event: MediaQueryListEvent) => onChange(event.matches);
  query.addEventListener("change", handle);
  return () => query.removeEventListener("change", handle);
}
