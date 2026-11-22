import "./page";

import React, { useMemo } from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import { CastContext } from "./avatars/cast";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { HostSetup } from "./components/HostSetup";
import { HostMonitor } from "./components/HostUpdates";
import { Roster } from "./components/Markdown";
import { SeasonalLooks } from "./components/Seasonal";
import { TokenEntry } from "./components/TokenEntry";
import { useStore } from "./lib/store";

if (import.meta.env.MODE === "probe") void import("./lib/probe").then((probe) => probe.install());

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

function Guaca() {
  const everyone = useStore((state) => state.agents);
  const roster = useMemo(() => everyone.map((agent) => agent.name), [everyone]);
  const cast = useStore((state) => state.prefs.cast);

  return (
    <Roster.Provider value={roster}>
      <CastContext.Provider value={cast}>
        <SeasonalLooks>
          <App />
        </SeasonalLooks>
      </CastContext.Provider>
    </Roster.Provider>
  );
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <ErrorBoundary>
      <HostSetup>
        <TokenEntry>
          <HostMonitor>
            <Guaca />
          </HostMonitor>
        </TokenEntry>
      </HostSetup>
    </ErrorBoundary>
  </React.StrictMode>,
);
