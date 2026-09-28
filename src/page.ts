/**
 * What every page of this app does before it draws anything.
 *
 * There are two: the window, and the panel under the menu bar icon. Both are
 * this bundle drawing a workspace, and anything one of them forgot here is a
 * panel offering the webview's Reload and Inspect Element on a right-click, or
 * a blank rectangle where a failure should have been written down. Imported
 * first, for its effects, by both entries.
 */

// Inter with its optical-size axis, so a title is drawn from the display
// cut and a line of text from the text cut without a second family. The
// other faces are registered here and fetched only if a preference names
// them: a font file is loaded when something is drawn in it, not before.
import "@fontsource-variable/inter/opsz.css";
import "@fontsource-variable/jetbrains-mono";
import "@fontsource-variable/atkinson-hyperlegible-next";
import "@fontsource-variable/atkinson-hyperlegible-mono";
import "@fontsource-variable/literata/opsz.css";
import "@fontsource-variable/literata/opsz-italic.css";

import { applyAppearance } from "./lib/appearance";
import { loadPrefs } from "./lib/prefs";
import "./styles.css";

/**
 * The operator's appearance, before anything is drawn.
 *
 * `App` applies these too, and has to: it is what a change while the window is
 * open goes through, and what follows the OS when the surface is set to. But an
 * effect runs after the first commit has painted, so doing it only there means
 * every launch shows one frame of white at 100% before snapping to whatever was
 * stored. One synchronous write here, before the root is created, and there is
 * nothing to snap from.
 */
applyAppearance(loadPrefs());

/**
 * Paints a failure that happened before or outside React.
 *
 * Without this, a module that throws on import leaves an empty document, which
 * renders as a blank window with no way to tell whether the app crashed or
 * simply has nothing to show.
 */
function reportFatal(message: string) {
  const root = document.getElementById("root");
  if (!root || root.childElementCount > 0) return;
  root.innerHTML = "";

  const wrap = document.createElement("div");
  wrap.style.cssText = "padding:2rem;max-width:48rem;margin:0 auto;font:14px/1.6 system-ui";

  const heading = document.createElement("h1");
  heading.textContent = "Guaca could not start";
  heading.style.cssText = "font-size:1rem;margin:0 0 .5rem";

  const detail = document.createElement("pre");
  detail.textContent = message;
  detail.style.cssText = "white-space:pre-wrap;opacity:.8;margin:0";

  wrap.append(heading, detail);
  root.append(wrap);
}

/**
 * The webview's own context menu is Reload and Inspect Element: developer
 * furniture that no operator wants and that leaks the fact this is a webview.
 *
 * Text fields keep theirs, because right-click is how you reach cut, copy and
 * paste. Anything with something better to offer, like an agent row in the
 * rail, handles the event itself and this listener never sees it.
 */
document.addEventListener("contextmenu", (event) => {
  const target = event.target as HTMLElement | null;
  if (target?.closest?.("input, textarea, [contenteditable='true']")) return;
  event.preventDefault();
});

window.addEventListener("error", (event) => reportFatal(event.message));
window.addEventListener("unhandledrejection", (event) =>
  reportFatal(String((event.reason as Error)?.message ?? event.reason)),
);
