import { useCallback, useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { relativeTime, useNow } from "../lib/time";
import { type AgentCard, errorMessage, type Signin } from "../lib/types";

interface Props {
  agent: AgentCard;
}

export function SigninList({ agent }: Props) {
  const [signins, setSignins] = useState<Signin[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const now = useNow(30_000);
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

  useEffect(() => {
    if (!somewhere) return;
    let canceled = false;
    void api
      .scanAgentSignins(agent.id)
      .then((found) => {
        if (!canceled) setSignins(found);
      })
      .catch(() => {});
    return () => {
      canceled = true;
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
  if (!somewhere) return null;

  return (
    <div className="access">
      <div className="routines__head">
        <span className="field__label">Signed in</span>
        <button
          type="button"
          className="btn btn--ghost btn--small"
          disabled={busy}
          onClick={() => void rescan()}
        >
          {busy ? "Checking…" : "Check now"}
        </button>
      </div>

      {signins.length === 0 && (
        <p className="field__hint">
          Nothing. Open this agent's browser or its computer's screen, sign in to a site there, and
          it will show up here and on every other agent's roster. You do not have to tell{" "}
          {agent.name} about it.
        </p>
      )}

      {signins.map((signin) => (
        <div className="access__item" key={`${signin.surface}:${signin.domain}`}>
          <div className="access__row">
            <strong className="access__name">{signin.service}</strong>
            <span
              className="access__account"
              title={
                signin.surface === "browser"
                  ? "In this agent's browser, which is what `browse` uses."
                  : "In the browser on this agent's computer screen, which only `use_screen` reaches."
              }
            >
              {signin.surface === "browser" ? "in its browser" : "on its screen"}
            </span>
            {!signin.recognized && (
              <span
                className="access__account"
                title="Matched by a session cookie on a site this browser has visited, rather than by a known signature."
              >
                looks signed in
              </span>
            )}
            <span className="access__when">seen {relativeTime(signin.lastSeenAt, now)} ago</span>
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
