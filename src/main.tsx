import "@fontsource-variable/inter";
import "@fontsource-variable/instrument-sans";
import "@fontsource-variable/jetbrains-mono";

import React, { useMemo } from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Roster } from "./components/Markdown";
import { applyAppearance } from "./lib/appearance";
import { loadPrefs } from "./lib/prefs";
import { useStore } from "./lib/store";
import "./styles.css";

/**
 * The operator's scale and surface, before anything is drawn.
 *
 * `App` applies these too, and has to: it is what a change while the window is
 * open goes through, and what follows the OS when the surface is set to. But an
 * effect runs after the first commit has painted, so doing it only there means
 * every launch shows one frame of white at 100% before snapping to whatever was
 * stored. One synchronous write here, before the root is created, and there is
 * nothing to snap from.
 */
const stored = loadPrefs();
applyAppearance(stored.uiScale, stored.surface);

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

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

/**
 * The names an `@` in a message body is allowed to resolve to.
 *
 * At the root because a body is drawn in a channel, in a pair's thread, in the
 * activity board and behind a search hit, and none of those should have to
 * remember to say so. The whole roster rather than the live one: a transcript
 * is history, and an agent that has since been let go was still an agent when
 * somebody wrote to it. The composer answers the other question, which is who
 * a message can be delivered to, so it completes against the live crew.
 */
function Guaca() {
  const everyone = useStore((state) => state.agents);
  const roster = useMemo(() => everyone.map((agent) => agent.name), [everyone]);

  return (
    <Roster.Provider value={roster}>
      <App />
    </Roster.Provider>
  );
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <ErrorBoundary>
      <Guaca />
    </ErrorBoundary>
  </React.StrictMode>,
);
