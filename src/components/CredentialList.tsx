import { useCallback, useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { type Connector, type ConnectorDraft, errorMessage, type GroupId } from "../lib/types";

interface Props {
  groupId: GroupId;
}

/**
 * The other half of what a crew can reach: a token, in a variable, on a machine.
 *
 * This used to lead with a grid of twelve brands, each of which filled in a
 * variable name and a note for a service Guaca knew nothing else about. The
 * grid is gone. What a crew reaches through a *plugin* is now a short list of
 * servers that publish their own tools and sign in for themselves, and the
 * twelve tiles were a worse version of that offer: a logo, and then four
 * questions the operator had to answer anyway.
 *
 * What is left is the escape hatch, and it stays because it is the only way to
 * reach a service with no plugin. It is also the only thing that can still read
 * the credentials a workspace already holds: deleting the form would leave
 * those rows on every machine in the group with nothing on screen to say so.
 *
 * Nothing here has ever held a credential's value. The backend returns whether
 * one is set and its last four characters, and there is no command that would
 * return more.
 */
export function CredentialList({ groupId }: Props) {
  const [connectors, setConnectors] = useState<Connector[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [draft, setDraft] = useState({ service: "", envVar: "" });
  const [secret, setSecret] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const serviceRef = useRef<HTMLInputElement>(null);

  // Opening the form is a commitment to fill it in, so the cursor goes to the
  // first field. Done with a ref rather than autoFocus so the timing is ours,
  // which is the same reason the agent editor does it this way.
  useEffect(() => {
    if (adding) serviceRef.current?.focus();
  }, [adding]);

  const load = useCallback(async () => {
    try {
      setConnectors(await api.groupConnectors(groupId));
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
      setConnectors([]);
    }
  }, [groupId]);

  useEffect(() => {
    void load();
  }, [load]);

  const reset = () => {
    setAdding(false);
    setDraft({ service: "", envVar: "" });
    setSecret("");
  };

  const run = async (action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      await load();
      return true;
    } catch (caught) {
      setError(errorMessage(caught));
      return false;
    } finally {
      setBusy(false);
    }
  };

  const ready = secret.trim() && draft.service.trim() && draft.envVar.trim();
  const add = () =>
    void run(() =>
      api.createConnector({
        groupId,
        service: draft.service,
        account: "",
        envVar: draft.envVar,
        note: "",
        secret,
      } satisfies ConnectorDraft),
    ).then((ok) => ok && reset());

  if (connectors === null) return <p className="field__hint">Loading credentials…</p>;

  return (
    <div className="access">
      <div className="routines__head">
        <span className="field__label">Credentials</span>
        {adding && (
          <button type="button" className="btn btn--ghost btn--small" onClick={reset}>
            Cancel
          </button>
        )}
      </div>

      {connectors.map((connector) => (
        <div className="access__item" key={connector.id}>
          <div className="access__row">
            <strong className="access__name">{connector.service}</strong>
            <span className="access__where">${connector.envVar}</span>
            <span className="access__when">
              {connector.secretSet ? connector.secretHint : "no value set"}
            </span>
            <button
              type="button"
              className="btn btn--small btn--ghost"
              disabled={busy}
              onClick={() => void run(() => api.deleteConnector(connector.id))}
            >
              Forget
            </button>
          </div>
          {connector.note && <p className="field__hint">{connector.note}</p>}
        </div>
      ))}

      {adding ? (
        <div className="access__item">
          <div className="access__row">
            <input
              className="input input--slim"
              placeholder="what it is for"
              ref={serviceRef}
              value={draft.service}
              onChange={(event) => setDraft({ ...draft, service: event.target.value })}
            />
            <input
              className="input input--slim input--mono"
              placeholder="MY_API_KEY"
              value={draft.envVar}
              onChange={(event) => setDraft({ ...draft, envVar: event.target.value })}
            />
          </div>
          <p className="field__hint">
            The variable name is what the agent will use, so give it the one the service's own
            documentation uses.
          </p>
          <div className="access__row">
            <input
              className="input input--mono"
              type="password"
              placeholder={`${draft.service.trim() || "the"} token`}
              value={secret}
              onChange={(event) => setSecret(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && ready) add();
              }}
            />
            <button
              type="button"
              className="btn btn--small btn--primary"
              // The value has to arrive now. There is no edit command to supply
              // one later, and a variable stored empty reads to the agent as a
              // revoked token rather than as unfinished setup.
              disabled={busy || !ready}
              onClick={add}
            >
              Add
            </button>
          </div>
        </div>
      ) : (
        <>
          <p className="field__hint">
            For a service with no plugin. Every machine in this group gets it as an environment
            variable, and the agents are told the name and told not to print it: the value never
            reaches the model. For anything you log into with a browser, just sign in on an agent's
            computer instead.
          </p>
          <button type="button" className="btn btn--small" onClick={() => setAdding(true)}>
            Add a credential
          </button>
        </>
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.4rem 0 0" }}>
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}
