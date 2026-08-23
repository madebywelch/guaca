import { useCallback, useEffect, useState } from "react";

import { api, openExternal } from "../lib/ipc";
import { BRANDS, hostOf } from "../lib/plugins";
import { errorMessage, type GroupId, type Plugin, type PluginOffer } from "../lib/types";

interface Props {
  groupId: GroupId;
}

/**
 * The plugins a crew has, and the ones it can have.
 *
 * A plugin is a server the operator signs in to once, on behalf of the whole
 * group, after which every agent in it can call that server's tools. Nothing is
 * pasted and nothing is put on a machine: the call is made by Guaca with the
 * group's own sign-in on it, so the agent never holds a token and has nothing
 * to leak.
 *
 * A short list, and that is the design rather than a starting point. The list
 * this replaced was twelve brands and a text box, which asked the operator for
 * four things about a token they had — the variable it belongs in, the account
 * it acts as, a note for the agent, and whether the service was worth wiring up
 * at all. A server that publishes its own tools answers all four itself.
 *
 * What is on the list comes from Rust, in order, and is drawn as it arrives: a
 * catalogue the webview sorted or filtered would be a second opinion about
 * which servers exist, and the runtime is the one that dials them.
 *
 * Connecting can take minutes, because part of it happens in a browser in front
 * of a person. The row says so while it waits: a spinner with no explanation on
 * a button that opened another window is how an operator ends up clicking it
 * twice.
 */
export function PluginList({ groupId }: Props) {
  const [offers, setOffers] = useState<PluginOffer[] | null>(null);
  const [connected, setConnected] = useState<Plugin[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [catalogue, held] = await Promise.all([
        api.pluginCatalogue(),
        api.groupPlugins(groupId),
      ]);
      setOffers(catalogue);
      setConnected(held);
      setError(null);
    } catch (caught) {
      setError(errorMessage(caught));
      setOffers([]);
      setConnected([]);
    }
  }, [groupId]);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (key: string, action: () => Promise<unknown>) => {
    setBusy(key);
    setError(null);
    try {
      await action();
      await load();
    } catch (caught) {
      setError(errorMessage(caught));
    } finally {
      setBusy(null);
    }
  };

  if (offers === null || connected === null) return <p className="field__hint">Loading plugins…</p>;

  return (
    <div className="access">
      {offers.map((offer) => {
        const held = connected.find((plugin) => plugin.kind === offer.kind);
        const brand = BRANDS[offer.kind];
        const working = busy === offer.kind;

        return (
          <div className="access__item" key={offer.kind}>
            <div className="access__row">
              <span
                className="mark"
                aria-hidden="true"
                style={{ "--mark": brand.color } as React.CSSProperties}
              >
                <svg viewBox="0 0 24 24" className="mark__icon" role="presentation">
                  <path d={brand.path} fill="currentColor" />
                </svg>
              </span>
              <strong className="access__name">{offer.name}</strong>
              {/* The host, not the whole URL: what matters before authorising
                  is which company is about to be handed the operator's
                  account, and the path is noise beside that. */}
              <span className="access__where">{hostOf(offer.endpoint)}</span>

              {held ? (
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  disabled={busy !== null}
                  onClick={() => void run(offer.kind, () => api.disconnectPlugin(held.id))}
                >
                  {working ? "Disconnecting…" : "Disconnect"}
                </button>
              ) : (
                <button
                  type="button"
                  className="btn btn--small btn--primary"
                  disabled={busy !== null}
                  onClick={() => void run(offer.kind, () => api.connectPlugin(groupId, offer.kind))}
                >
                  {working ? "Waiting for your browser…" : "Connect"}
                </button>
              )}
            </div>

            {held ? (
              <p className="field__hint">
                {held.tools.length} tool{held.tools.length === 1 ? "" : "s"}, offered to every agent
                in this group.
                {held.account && ` Signed in as ${held.account}.`}
                {!held.signedIn && " This server asked for no sign-in, so nothing was authorised."}
              </p>
            ) : (
              <p className="field__hint">
                {offer.blurb}{" "}
                <a
                  href={offer.docs}
                  target="_blank"
                  rel="noreferrer"
                  className="access__docs"
                  // Opened by the shell rather than in the webview, which has
                  // no way back. The href stays so the address is readable and
                  // copyable before it is followed.
                  onClick={(event) => {
                    event.preventDefault();
                    void openExternal(offer.docs);
                  }}
                >
                  What this can do
                </a>
              </p>
            )}
          </div>
        );
      })}

      {busy !== null && (
        <p className="field__hint">
          Finish in the browser window that just opened. Guaca is waiting, and gives up after five
          minutes.
        </p>
      )}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.4rem 0 0" }}>
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}
