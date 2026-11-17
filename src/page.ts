import "@fontsource-variable/inter/opsz.css";
import "@fontsource-variable/jetbrains-mono";
import "@fontsource-variable/atkinson-hyperlegible-next";
import "@fontsource-variable/atkinson-hyperlegible-mono";
import "@fontsource-variable/literata/opsz.css";
import "@fontsource-variable/literata/opsz-italic.css";

import { applyAppearance } from "./lib/appearance";
import { loadPrefs } from "./lib/prefs";
import "./styles.css";
import "./signal-layout.css";

applyAppearance(loadPrefs());

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

document.addEventListener("contextmenu", (event) => {
  const target = event.target as HTMLElement | null;
  if (target?.closest?.("input, textarea, [contenteditable='true']")) return;
  event.preventDefault();
});

window.addEventListener("error", (event) => reportFatal(event.message));
window.addEventListener("unhandledrejection", (event) =>
  reportFatal(String((event.reason as Error)?.message ?? event.reason)),
);
