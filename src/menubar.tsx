import "./page";

import React, { useEffect, useMemo, useState } from "react";
import ReactDOM from "react-dom/client";

import { CastContext } from "./avatars/cast";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Roster } from "./components/Markdown";
import { MenubarPanel } from "./components/MenubarPanel";
import { applyAppearance, watchSystemSurface } from "./lib/appearance";
import { api } from "./lib/ipc";
import { watchKeyboard } from "./lib/keyboard";
import { loadPrefs } from "./lib/prefs";
import { useRuntime } from "./lib/runtime";
import { useStore } from "./lib/store";
import { adoptRemote, guardPanelHost, UNAUTHORIZED_EVENT } from "./lib/transport";
import { errorMessage } from "./lib/types";

if (import.meta.env.MODE === "probe") void import("./lib/probe").then((probe) => probe.install());

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

watchKeyboard(window);

function Panel() {
  const everyone = useStore((state) => state.agents);
  const roster = useMemo(() => everyone.map((agent) => agent.name), [everyone]);
  const cast = useStore((state) => state.prefs.cast);
  const ready = useRuntime();
  const [refused, setRefused] = useState(false);

  useEffect(() => {
    const turnedAway = () => setRefused(true);
    window.addEventListener(UNAUTHORIZED_EVENT, turnedAway);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, turnedAway);
  }, []);

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
    return <Nowhere text="This host stopped accepting Guaca's token. Open Guaca to reconnect." />;
  }
  return (
    <Roster.Provider value={roster}>
      <CastContext.Provider value={cast}>
        <MenubarPanel ready={ready} />
      </CastContext.Provider>
    </Roster.Provider>
  );
}

function Nowhere({ text }: { text: string }) {
  return (
    <div className="menubar">
      <p className="menubar__note">{text}</p>
      <footer className="menubar__foot">
        <button
          type="button"
          className="btn btn--compact"
          onClick={() => void api.openWindow(null)}
        >
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
  .then(async (host) => {
    if (!host) {
      draw(<Nowhere text="Guaca is not connected to a host yet. Open it to choose one." />);
      return;
    }
    adoptRemote(host);
    if (!(await api.reportHost(host, host.generation))) return;
    guardPanelHost(host, () => api.reportHost(host, host.generation));
    draw(<Panel />);
  })
  .catch((error) => draw(<Nowhere text={`The panel could not start: ${errorMessage(error)}`} />));
