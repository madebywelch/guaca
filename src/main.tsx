import "./page";

import React, { useMemo } from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import { CastContext } from "./avatars/cast";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { HostSetup } from "./components/HostSetup";
import { HostMonitor } from "./components/HostUpdates";
import { Roster } from "./components/Markdown";
import { TokenEntry } from "./components/TokenEntry";
import { useStore } from "./lib/store";

const root = document.getElementById("root");
if (!root) throw new Error("missing #root");

/**
 * The names an `@` in a message body is allowed to resolve to.
 *
 * At the root because a body is drawn in a channel, in a pair's thread, on the
 * flow board and behind a search hit, and none of those should have to remember
 * to say so. The whole roster rather than the live one: a transcript
 * is history, and an agent that has since been let go was still an agent when
 * somebody wrote to it. The composer answers the other question, which is who
 * a message can be delivered to, so it completes against the live crew.
 *
 * The cast is here for the same reason: an agent is drawn on fifteen surfaces,
 * and every one of them draws it the way the operator chose.
 */
function Guaca() {
  const everyone = useStore((state) => state.agents);
  const roster = useMemo(() => everyone.map((agent) => agent.name), [everyone]);
  const cast = useStore((state) => state.prefs.cast);

  return (
    <Roster.Provider value={roster}>
      <CastContext.Provider value={cast}>
        <App />
      </CastContext.Provider>
    </Roster.Provider>
  );
}

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <ErrorBoundary>
      {/* Outside `Guaca` rather than inside `App`: a browser without its
          token has no roster to provide and must make no call that would
          need one. A desktop passes straight through. */}
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
