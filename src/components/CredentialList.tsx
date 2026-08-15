import { useCallback, useEffect, useRef, useState } from "react";

import { CATALOG, type CatalogEntry, entryFor } from "../lib/connectorCatalog";
import { api } from "../lib/ipc";
import { type Connector, type ConnectorDraft, errorMessage, type GroupId } from "../lib/types";

interface Props {
  groupId: GroupId;
}

/** The service being added, or `custom` for one the catalog does not list. */
type Picked = CatalogEntry | "custom" | null;

/**
 * The API credentials a crew holds, managed where they belong: on the group.
 *
 * Every machine in the group is handed these as environment variables, so this
 * is a property of the crew rather than of any one agent. The other way an
 * agent reaches an account, a browser that is already logged in, is not managed
 * anywhere: it is detected from the machine and shown on the agent instead.
 *
 * Adding one is a service and a token, in that order. Everything else about a
 * GitHub credential is already known once you have said GitHub: the variable it
 * belongs in is `GITHUB_TOKEN` on every machine anywhere, and asking the
 * operator to type that, plus an account name, plus a note, is four questions
 * to collect one answer they actually have.
 *
 * Nothing here has ever held a credential's value. The backend returns whether
 * one is set and its last four characters, and there is no command that would
 * return more.
 */
export function CredentialList({ groupId }: Props) {
  const [connectors, setConnectors] = useState<Connector[] | null>(null);
  const [picked, setPicked] = useState<Picked>(null);
  const [secret, setSecret] = useState("");
  const [custom, setCustom] = useState({ service: "", envVar: "" });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const secretRef = useRef<HTMLInputElement>(null);

  // Picking a service is a commitment to fill in the one field it revealed, so
  // the cursor goes there. Done with a ref rather than autoFocus so the timing
  // is ours, which is the same reason the agent editor does it this way.
  useEffect(() => {
    if (picked !== null) secretRef.current?.focus();
  }, [picked]);

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
    setPicked(null);
    setSecret("");
    setCustom({ service: "", envVar: "" });
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

  if (connectors === null) return <p className="field__hint">Loading credentials…</p>;

  const held = new Set(connectors.map((connector) => connector.service));
  const offered = CATALOG.filter((entry) => !held.has(entry.service));
  const service = picked === "custom" ? custom.service : (picked?.service ?? "");
  const envVar = picked === "custom" ? custom.envVar : (picked?.envVar ?? "");

  return (
    <div className="connectors">
      <div className="routines__head">
        <span className="field__label">Credentials</span>
        {picked !== null && (
          <button type="button" className="btn btn--ghost btn--small" onClick={reset}>
            Cancel
          </button>
        )}
      </div>

      {connectors.map((connector) => (
        <div className="connector" key={connector.id}>
          <div className="connector__row">
            <Mark entry={entryFor(connector.service)} fallback={connector.service} />
            <strong className="connector__service">{connector.service}</strong>
            <span className="connector__where">${connector.envVar}</span>
            <span className="connector__when">
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

      {picked === null ? (
        <>
          <p className="field__hint">
            Pick a service and paste its token. Every machine in this group gets it as an
            environment variable, and the agents are told the name and told not to print it: the
            value never reaches the model. For anything you log into with a browser, just sign in on
            an agent's computer instead.
          </p>
          <div className="services">
            {offered.map((entry) => (
              <button
                key={entry.service}
                type="button"
                className="service"
                style={{ "--mark": entry.color } as React.CSSProperties}
                onClick={() => setPicked(entry)}
              >
                <Mark entry={entry} fallback={entry.service} />
                <span className="service__name">{entry.service}</span>
              </button>
            ))}
            <button
              type="button"
              className="service service--other"
              onClick={() => setPicked("custom")}
            >
              <span className="mark mark--other" aria-hidden="true">
                +
              </span>
              <span className="service__name">Something else</span>
            </button>
          </div>
        </>
      ) : (
        <div className="connector">
          {picked === "custom" ? (
            <>
              <div className="connector__row">
                <input
                  className="input input--slim"
                  placeholder="what it is for"
                  value={custom.service}
                  onChange={(event) => setCustom({ ...custom, service: event.target.value })}
                />
                <input
                  className="input input--slim input--mono"
                  placeholder="MY_API_KEY"
                  value={custom.envVar}
                  onChange={(event) => setCustom({ ...custom, envVar: event.target.value })}
                />
              </div>
              <p className="field__hint">
                The variable name is what the agent will use, so give it the one the service's own
                documentation uses.
              </p>
            </>
          ) : (
            <>
              <div className="connector__row">
                <Mark entry={picked} fallback={picked.service} />
                <strong className="connector__service">{picked.service}</strong>
                <span className="connector__where">${picked.envVar}</span>
              </div>
              <p className="field__hint">Get one at {picked.where}.</p>
            </>
          )}
          <div className="connector__row">
            <input
              className="input input--mono"
              type="password"
              placeholder={`${service || "the"} token`}
              value={secret}
              ref={secretRef}
              onChange={(event) => setSecret(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && secret.trim() && service && envVar) {
                  void run(() =>
                    api.createConnector({
                      groupId,
                      service,
                      account: "",
                      envVar,
                      note: picked === "custom" ? "" : (picked.note ?? ""),
                      secret,
                    } satisfies ConnectorDraft),
                  ).then((ok) => ok && reset());
                }
              }}
            />
            <button
              type="button"
              className="btn btn--small btn--primary"
              // The value has to arrive now. There is no edit command to supply
              // one later, and a variable stored empty reads to the agent as a
              // revoked token rather than as unfinished setup.
              disabled={busy || !secret.trim() || !service.trim() || !envVar.trim()}
              onClick={() =>
                void run(() =>
                  api.createConnector({
                    groupId,
                    service,
                    account: "",
                    envVar,
                    note: picked === "custom" ? "" : (picked.note ?? ""),
                    secret,
                  } satisfies ConnectorDraft),
                ).then((ok) => ok && reset())
              }
            >
              Add
            </button>
          </div>
        </div>
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.4rem 0 0" }}>
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}

/**
 * A service's tile: its own mark, in its own colour.
 *
 * The mark is real path data rather than something drawn by eye, because a logo
 * approximated at twenty pixels is just a wrong logo. The few brands with no
 * published icon fall back to an initial, which is why the tile is a neutral
 * chip carrying a coloured glyph rather than a coloured chip: the two kinds sit
 * in one grid without one of them looking like a mistake.
 */
function Mark({ entry, fallback }: { entry?: CatalogEntry; fallback: string }) {
  return (
    <span
      className="mark"
      aria-hidden="true"
      style={{ "--mark": entry?.color ?? "var(--muted)" } as React.CSSProperties}
    >
      {entry?.path ? (
        <svg viewBox="0 0 24 24" className="mark__icon" role="presentation">
          <path d={entry.path} fill="currentColor" />
        </svg>
      ) : (
        (entry?.mark ?? fallback.slice(0, 1).toUpperCase())
      )}
    </span>
  );
}
