import "@fontsource-variable/inter";
import "@fontsource-variable/space-grotesk";
import "@fontsource-variable/jetbrains-mono";

import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import "./styles.css";

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
  heading.textContent = "Guac could not start";
  heading.style.cssText = "font-size:1rem;margin:0 0 .5rem";

  const detail = document.createElement("pre");
  detail.textContent = message;
  detail.style.cssText = "white-space:pre-wrap;opacity:.8;margin:0";

  wrap.append(heading, detail);
  root.append(wrap);
}

window.addEventListener("error", (event) => reportFatal(event.message));
window.addEventListener("unhandledrejection", (event) =>
  reportFatal(String((event.reason as Error)?.message ?? event.reason)),
);

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
