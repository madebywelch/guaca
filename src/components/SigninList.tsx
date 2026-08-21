import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { relativeTime, useNow } from "../lib/time";
import { type AgentCard, errorMessage, type Signin } from "../lib/types";

interface Props {
  agent: AgentCard;
}

/**
 * What an agent is signed in to, and where. Read, not written.
 *
 * There is nothing to fill in here on purpose. Whatever holds the cookies
 * knows, so Guaca asks it rather than asking the operator to keep a list up to
 * date: sign in and it appears, log out and it goes. The list is also on every
 * peer's roster, which is what lets a crew route work to the one agent that can
 * do it.
 *
 * Each row says which of the agent's two places holds the session, and that is
 * not decoration. A computer and a browser have unrelated cookie jars, so an
 * operator looking at "LinkedIn" needs to know which window the agent will find
 * it in, and which one to sign in through next time.
 *
 * The scan runs when this opens and again whenever the agent has been working,
 * so the usual case is that it is already right by the time anyone looks.
 */
export function SigninList({ agent }: Props) {
  const [signins, setSignins] = useState<Signin[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const now = useNow(30_000);
  // Whether there is anything to ask. Either place can hold a session, so an
  // agent that has only ever used the web is still worth scanning.
  const somewhere = Boolean(agent.sandboxId || agent.browserId);

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
    if (!somewhere) return;
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
  }, [agent.id, somewhere]);

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
          disabled={busy || !somewhere}
          onClick={() => void rescan()}
        >
          {busy ? "Checking…" : "Check now"}
        </button>
      </div>

      {!somewhere && (
        <p className="field__hint">
          {agent.name} has no computer and no browser yet, so there is nothing holding a session.
        </p>
      )}

      {somewhere && signins.length === 0 && (
        <p className="field__hint">
          Nothing. Open this agent's browser or its computer's screen, sign in to a site there, and
          it will show up here and on every other agent's roster. You do not have to tell{" "}
          {agent.name} about it.
        </p>
      )}

      {signins.map((signin) => (
        <div className="connector" key={`${signin.surface}:${signin.domain}`}>
          <div className="connector__row">
            <strong className="connector__service">{signin.service}</strong>
            <span
              className="connector__account"
              title={
                signin.surface === "browser"
                  ? "In this agent's browser, which is what `browse` uses."
                  : "In the browser on this agent's computer screen, which only `use_screen` reaches."
              }
            >
              {signin.surface === "browser" ? "in its browser" : "on its screen"}
            </span>
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
