import "./page";

import React, { useEffect, useMemo, useState } from "react";
import ReactDOM from "react-dom/client";

import { CastContext } from "./avatars/cast";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Roster } from "./components/Markdown";
import { MenubarPanel } from "./components/MenubarPanel";
import { applyAppearance, watchSystemSurface } from "./lib/appearance";
import { api } from "./lib/ipc";
import { loadPrefs } from "./lib/prefs";
import { useRuntime } from "./lib/runtime";
import { useStore } from "./lib/store";
import { adoptRemote, UNAUTHORIZED_EVENT } from "./lib/transport";
import { errorMessage } from "./lib/types";

/**
 * The panel under the menu bar icon, as a page.
 *
 * A second client of whichever host the window is attached to, told which one
 * by the tray rather than reading it from storage: the tray creates this page
 * only once the window has said, and reloads it when the window says something
 * else, so the answer and the moment to act on it arrive together.
 */

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

/** A panel with somewhere to be, or the reason it has nowhere. */
function Panel() {
  const everyone = useStore((state) => state.agents);
  const roster = useMemo(() => everyone.map((agent) => agent.name), [everyone]);
  const cast = useStore((state) => state.prefs.cast);
  const ready = useRuntime();
  const [refused, setRefused] = useState(false);

  // Turned away is the window's problem to solve, because the window is where
  // a token is pasted. The panel says so and waits to be reloaded with the
  // one that works.
  useEffect(() => {
    const turnedAway = () => setRefused(true);
    window.addEventListener(UNAUTHORIZED_EVENT, turnedAway);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, turnedAway);
  }, []);

  // The operator's appearance is chosen in the window and stored where this
  // page can read it, so it is read again every time the panel is shown
  // rather than kept from whenever this page loaded. Read, never written: the
  // window owns the preferences.
  useEffect(() => {
    const shown = () => {
      const prefs = loadPrefs();
      useStore.setState({ prefs });
      applyAppearance(prefs);
    };
    window.addEventListener("focus", shown);
    const unwatch = watchSystemSurface(() => applyAppearance(useStore.getState().prefs));
    return () => {
      window.removeEventListener("focus", shown);
      unwatch();
    };
  }, []);

  if (refused) {
    return (
      <Nowhere text="This workspace stopped accepting Guaca's token. Open Guaca to reconnect." />
    );
  }
  return (
    <Roster.Provider value={roster}>
      <CastContext.Provider value={cast}>
        <MenubarPanel ready={ready} />
      </CastContext.Provider>
    </Roster.Provider>
  );
}

/** The panel with no workspace to show, and the one way to get one. */
function Nowhere({ text }: { text: string }) {
  return (
    <div className="menubar">
      <p className="menubar__note">{text}</p>
      <footer className="menubar__foot">
        <button type="button" className="btn btn--small" onClick={() => void api.openWindow(null)}>
          Open Guaca
        </button>
      </footer>
    </div>
  );
}

const draw = (panel: React.ReactNode) =>
  ReactDOM.createRoot(root).render(
    <React.StrictMode>
      <ErrorBoundary>{panel}</ErrorBoundary>
    </React.StrictMode>,
  );

api
  .menubarHost()
  .then((host) => {
    if (!host) {
      draw(<Nowhere text="Guaca is not connected to a workspace yet. Open it to choose one." />);
      return;
    }
    adoptRemote(host);
    draw(<Panel />);
  })
  .catch((error) => draw(<Nowhere text={`The panel could not start: ${errorMessage(error)}`} />));
