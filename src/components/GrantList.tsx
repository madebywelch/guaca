import { useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { type AgentCard, errorMessage, type ProtectedAction } from "../lib/types";

interface Props {
  agent: AgentCard;
}

/** What each standing grant lets the agent do, in the operator's words. */
const PHRASE: Record<ProtectedAction, string> = {
  createAgent: "Adds agents to this workspace without asking.",
  // Not offered on the request itself, since a standing yes would cover every
  // future send and purchase rather than the one being asked about. Listed
  // anyway: a grant that exists and cannot be seen is worse than one that can.
  actOnBehalf: "Acts outside this workspace in your name without asking.",
};

/**
 * Standing permissions, and the way to take one back.
 *
 * A grant is created by clicking "Always allow" on a request in the transcript,
 * which is a decision made in one second about every future request. This is
 * where that decision stops being permanent. A permission that could only ever
 * be given would make the middle button on every request a thing to think hard
 * about, which is the opposite of what it is for.
 *
 * Nothing is drawn when there are none: an empty panel here would advertise a
 * mechanism most agents never touch.
 */
export function GrantList({ agent }: Props) {
  const [grants, setGrants] = useState<ProtectedAction[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<ProtectedAction | null>(null);

  useEffect(() => {
    let cancelled = false;
    void api
      .agentGrants(agent.id)
      .then((found) => {
        if (!cancelled) setGrants(found);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
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
    <div className="connectors">
      <div className="routines__head">
        <span className="field__label">Standing permission</span>
      </div>

      {grants.map((action) => (
        <div className="connector" key={action}>
          <div className="connector__row">
            <strong className="connector__service">{PHRASE[action]}</strong>
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
