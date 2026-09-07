import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import {
  type AgentCard,
  type AgentId,
  type Connector,
  errorMessage,
  type GroupId,
} from "../lib/types";

interface Props {
  groupId: GroupId;
  crew?: Pick<AgentCard, "id" | "name">[];
}

/** Values are write-only. Grants and rotation take effect on the next command or job. */
export function CredentialList({ groupId, crew = [] }: Props) {
  const [connectors, setConnectors] = useState<Connector[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState({ service: "", envVar: "" });
  const [secret, setSecret] = useState("");
  const [agents, setAgents] = useState<AgentId[]>([]);
  const [editing, setEditing] = useState<Connector | null>(null);
  const [replacement, setReplacement] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const serviceRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (adding) serviceRef.current?.focus();
  }, [adding]);

  const load = useCallback(async () => {
    const listed = await api.groupConnectors(groupId);
    setConnectors(listed);
  }, [groupId]);

  useEffect(() => {
    void load().catch((caught) => setError(errorMessage(caught)));
  }, [load]);

  const reset = () => {
    setAdding(false);
    setEditing(null);
    setDraft({ service: "", envVar: "" });
    setSecret("");
    setReplacement("");
    setAgents([]);
  };

  const run = async (action: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      await action();
      reset();
      await load();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(false);
    }
  };

  const recipients = (selected: AgentId[], change: (ids: AgentId[]) => void) => (
    <div className="field">
      <span className="field__label">Agents with access</span>
      <fieldset className="choices secret-recipients" aria-label="Agents with access">
        {crew.map((agent) => (
          <button
            type="button"
            className="choice"
            key={agent.id}
            disabled={busy}
            aria-pressed={selected.includes(agent.id)}
            onClick={() =>
              change(
                selected.includes(agent.id)
                  ? selected.filter((id) => id !== agent.id)
                  : [...selected, agent.id],
              )
            }
          >
            {agent.name}
          </button>
        ))}
      </fieldset>
      <span className="field__hint">
        {selected.length === 0 ? "Nobody has access." : `${selected.length} selected.`}
      </span>
    </div>
  );

  return (
    <div className="access">
      <p className="field__hint">
        Use the service’s variable name, such as CLOUDFLARE_API_TOKEN. Values are never shown after
        saving. Changes apply to new commands and jobs; stop an existing job before revoking its
        access. Repository authentication is managed under Repositories.
      </p>
      {connectors === null && !error && <p className="field__hint">Loading secrets…</p>}
      {connectors?.map((connector) => (
        <div className="access__item" key={connector.id}>
          <div className="access__row">
            <strong className="access__name">{connector.service}</strong>
            <span className="access__where">${connector.envVar}</span>
            <span className="access__when">
              {connector.secretSet ? "Value saved" : "no value set"}
            </span>
            <button
              type="button"
              className="btn btn--small"
              disabled={busy || adding}
              onClick={() => {
                setEditing(connector);
                setAgents(connector.agents);
                setReplacement("");
              }}
            >
              Manage
            </button>
            <button
              type="button"
              className="btn btn--small btn--ghost"
              disabled={busy}
              onClick={() => void run(() => api.deleteConnector(connector.id))}
            >
              Forget
            </button>
          </div>
          <p className="field__hint">
            {connector.agents.length === 0
              ? "No agents have access"
              : crew
                  .filter((agent) => connector.agents.includes(agent.id))
                  .map((agent) => agent.name)
                  .join(", ")}
          </p>
          {connector.note && <p className="field__hint">{connector.note}</p>}
          {editing?.id === connector.id && (
            <>
              {recipients(agents, setAgents)}
              <label className="field">
                <span className="field__label">Replace value (optional)</span>
                <input
                  className="input input--mono"
                  type="password"
                  autoComplete="new-password"
                  value={replacement}
                  onChange={(event) => setReplacement(event.target.value)}
                />
              </label>
              <div className="access__row">
                <button
                  type="button"
                  className="btn btn--small"
                  disabled={busy || (replacement.length > 0 && !replacement.trim())}
                  onClick={() =>
                    void run(() => api.updateConnector(connector.id, agents, replacement || null))
                  }
                >
                  Save changes
                </button>
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  disabled={busy}
                  onClick={reset}
                >
                  Cancel
                </button>
              </div>
            </>
          )}
        </div>
      ))}

      {adding ? (
        <div className="access__item">
          <label className="field">
            <span className="field__label">Service</span>
            <input
              className="input"
              placeholder="what it is for"
              ref={serviceRef}
              value={draft.service}
              onChange={(event) => setDraft({ ...draft, service: event.target.value })}
            />
          </label>
          <label className="field">
            <span className="field__label">Environment variable</span>
            <input
              className="input input--mono"
              placeholder="MY_API_KEY"
              value={draft.envVar}
              onChange={(event) => setDraft({ ...draft, envVar: event.target.value })}
            />
          </label>
          <label className="field">
            <span className="field__label">Secret value</span>
            <input
              className="input input--mono"
              type="password"
              autoComplete="new-password"
              placeholder={`${draft.service.trim() || "the"} token`}
              value={secret}
              onChange={(event) => setSecret(event.target.value)}
            />
          </label>
          {recipients(agents, setAgents)}
          <div className="access__row">
            <button
              type="button"
              className="btn btn--small btn--primary"
              disabled={busy || !secret.trim() || !draft.service.trim() || !draft.envVar.trim()}
              onClick={() =>
                void run(() =>
                  api.createConnector({ groupId, ...draft, account: "", note: "", secret, agents }),
                )
              }
            >
              Add
            </button>
            <button
              type="button"
              className="btn btn--small btn--ghost"
              disabled={busy}
              onClick={reset}
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <button
          type="button"
          className="btn btn--small"
          disabled={busy || editing !== null || connectors === null}
          onClick={() => setAdding(true)}
        >
          Add a secret
        </button>
      )}

      {error && (
        <div className="banner banner--error" role="alert">
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}
