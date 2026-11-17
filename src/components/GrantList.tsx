import { useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { type AgentCard, errorMessage, type ProtectedAction } from "../lib/types";

interface Props {
  agent: AgentCard;
}

const PHRASE: Record<ProtectedAction, string> = {
  createAgent: "Adds agents to this group without asking.",
  actOnBehalf: "Acts outside Guaca in your name without asking.",
  changeSettings: "Changes the app's settings without asking.",
};

export function GrantList({ agent }: Props) {
  const [grants, setGrants] = useState<ProtectedAction[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<ProtectedAction | null>(null);

  useEffect(() => {
    let canceled = false;
    void api
      .agentGrants(agent.id)
      .then((found) => {
        if (!canceled) setGrants(found);
      })
      .catch(() => {});
    return () => {
      canceled = true;
    };
  }, [agent.id]);

  const revoke = async (action: ProtectedAction) => {
    setBusy(action);
    setError(null);
    try {
      setGrants(await api.revokeGrant(agent.id, action));
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(null);
    }
  };

  if (grants.length === 0) return null;

  return (
    <div className="access">
      <div className="routines__head">
        <span className="field__label">Standing permission</span>
      </div>

      {grants.map((action) => (
        <div className="access__item" key={action}>
          <div className="access__row">
            <strong className="access__name">{PHRASE[action]}</strong>
            <button
              type="button"
              className="btn btn--ghost btn--small"
              style={{ marginLeft: "auto" }}
              disabled={busy !== null}
              onClick={() => void revoke(action)}
            >
              {busy === action ? "Removing…" : "Ask me again"}
            </button>
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
