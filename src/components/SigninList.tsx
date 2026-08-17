import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { relativeTime, useNow } from "../lib/time";
import { type AgentCard, errorMessage, type Signin } from "../lib/types";

interface Props {
  agent: AgentCard;
}

/**
 * What an agent's browser is signed in to. Read, not written.
 *
 * There is nothing to fill in here on purpose. The browser is holding the
 * cookies, so Guaca asks the machine rather than asking the operator to keep a
 * list up to date: sign in on the agent's screen and it appears, log out and it
 * goes. The list is also on every peer's roster, which is what lets a crew
 * route work to the one machine that can do it.
 *
 * The scan runs when this opens and again whenever the agent has been browsing,
 * so the usual case is that it is already right by the time anyone looks.
 */
export function SigninList({ agent }: Props) {
  const [signins, setSignins] = useState<Signin[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const now = useNow(30_000);

  const load = useCallback(async () => {
    try {
      setSignins(await api.agentSignins(agent.id));
    } catch (caught) {
      setError(errorMessage(caught));
      setSignins([]);
    }
  }, [agent.id]);

  useEffect(() => {
    void load();
  }, [load]);

  // Asking the machine costs a round trip, so the stored answer is drawn first
  // and corrected a moment later rather than leaving the panel empty.
  useEffect(() => {
    if (!agent.computerId) return;
    let cancelled = false;
    void api
      .scanAgentSignins(agent.id)
      .then((found) => {
        if (!cancelled) setSignins(found);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [agent.id, agent.computerId]);

  const rescan = async () => {
    setBusy(true);
    setError(null);
    try {
      setSignins(await api.scanAgentSignins(agent.id));
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  if (signins === null) return <p className="field__hint">Loading sessions…</p>;

  return (
    <div className="connectors">
      <div className="routines__head">
        <span className="field__label">Signed in</span>
        <button
          type="button"
          className="btn btn--ghost btn--small"
          disabled={busy || !agent.computerId}
          onClick={() => void rescan()}
        >
          {busy ? "Checking…" : "Check now"}
        </button>
      </div>

      {!agent.computerId && (
        <p className="field__hint">
          {agent.name} has no computer yet, so there is no browser to be signed in to.
        </p>
      )}

      {agent.computerId && signins.length === 0 && (
        <p className="field__hint">
          Nothing. Open this agent's computer, sign in to a site on its screen, and it will show up
          here and on every other agent's roster. You do not have to tell {agent.name} about it.
        </p>
      )}

      {signins.map((signin) => (
        <div className="connector" key={signin.domain}>
          <div className="connector__row">
            <strong className="connector__service">{signin.service}</strong>
            {!signin.recognised && (
              <span
                className="connector__account"
                title="Matched by a session cookie on a site this browser has visited, rather than by a known signature."
              >
                looks signed in
              </span>
            )}
            <span className="connector__when">seen {relativeTime(signin.lastSeenAt, now)} ago</span>
          </div>
        </div>
      ))}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.4rem 0 0" }}>
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}
