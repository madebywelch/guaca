import { useCallback, useEffect, useState } from "react";

import { api, openExternal } from "../lib/ipc";
import { BRANDS, hostOf } from "../lib/plugins";
import {
  type AgentCard,
  type AgentId,
  errorMessage,
  type GroupId,
  type Plugin,
  type PluginAccess,
  type PluginOffer,
} from "../lib/types";

interface Props {
  groupId: GroupId;
  /** The group's live agents, in rail order, for the ones a plugin is for. */
  crew: AgentCard[];
}

/**
 * The two answers to who may use a plugin.
 *
 * Two buttons rather than a list of agents with an "everyone" entry at the top:
 * everyone is a standing answer that covers whoever is hired next week, and a
 * tick beside today's names cannot say that.
 */
const ACCESS_MODES = [
  { value: "everyone", label: "Every agent" },
  { value: "chosen", label: "Only chosen agents" },
] as const;

/** The chosen set with one agent added or taken out. */
function toggled(access: PluginAccess, agent: AgentId): AgentId[] {
  const current = access.mode === "chosen" ? access.agents : [];
  return current.includes(agent) ? current.filter((id) => id !== agent) : [...current, agent];
}

/** Who a connected plugin is offered to, in the operator's words. */
function offeredTo(access: PluginAccess, crew: AgentCard[]): string {
  if (access.mode === "everyone") return "offered to every agent in this group";
  const named = crew.filter((agent) => access.agents.includes(agent.id));
  if (named.length === 0) return "offered to nobody: tick an agent, or nothing here can be called";
  return `offered to ${named.map((agent) => agent.name).join(", ")}`;
}

/**
 * The plugins a crew has, and the ones it can have.
 *
 * A plugin is a server the operator signs in to once, on behalf of the whole
 * group, after which the agents they chose can call that server's tools.
 * Nothing is pasted and nothing is put on a machine: the call is made by Guaca
 * with the group's own sign-in on it, so the agent never holds a token and has
 * nothing to leak.
 *
 * Signing in and handing it out are two decisions, and the second one is on
 * this row. Every agent is the default and the usual answer; the crew's Stripe
 * account is why it is not the only one. Each change is written the moment it
 * is made, like connecting and disconnecting above it: there is no Save on this
 * panel, so a draft nobody submitted would be a permission the operator thinks
 * they granted.
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
export function PluginList({ groupId, crew }: Props) {
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
        const chosen = held?.access.mode === "chosen" ? held.access.agents : [];

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
              <>
                <p className="field__hint">
                  {held.tools.length} tool{held.tools.length === 1 ? "" : "s"},{" "}
                  {offeredTo(held.access, crew)}.{held.account && ` Signed in as ${held.account}.`}
                  {!held.signedIn &&
                    " This server asked for no sign-in, so nothing was authorised."}
                </p>

                {/* Two buttons rather than a list with an "all" entry at the
                    top: every agent is a standing answer that covers whoever
                    is hired next week, and a tick beside today's names cannot
                    say that. */}
                <span className="field__label">Who can use it</span>
                <div className="choices">
                  {ACCESS_MODES.map((mode) => (
                    <button
                      key={mode.value}
                      type="button"
                      className="choice"
                      aria-pressed={held.access.mode === mode.value}
                      disabled={busy !== null}
                      onClick={() => {
                        // Narrowing to nobody is a real state and the hint
                        // above says so. Starting from the whole crew ticked
                        // would be a click that changed nothing, on the button
                        // whose whole purpose is to take something away.
                        if (held.access.mode === mode.value) return;
                        void run(`${offer.kind}-access`, () =>
                          api.setPluginAccess(
                            held.id,
                            mode.value === "everyone"
                              ? { mode: "everyone" }
                              : { mode: "chosen", agents: [] },
                          ),
                        );
                      }}
                    >
                      {mode.label}
                    </button>
                  ))}
                </div>

                {held.access.mode === "chosen" && (
                  <div className="choices">
                    {crew.length === 0 ? (
                      <span className="field__hint">This group has no agents yet.</span>
                    ) : (
                      crew.map((agent) => {
                        const has = chosen.includes(agent.id);
                        return (
                          <button
                            key={agent.id}
                            type="button"
                            className="choice"
                            // A toggle button rather than a checkbox with a
                            // name beside it: the row is a line of names and
                            // the state is which of them are lit, which is what
                            // pressed means. The same control the surface and
                            // the accent pickers use.
                            aria-pressed={has}
                            disabled={busy !== null}
                            onClick={() =>
                              void run(`${offer.kind}-${agent.id}`, () =>
                                api.setPluginAccess(held.id, {
                                  mode: "chosen",
                                  agents: toggled(held.access, agent.id),
                                }),
                              )
                            }
                          >
                            {agent.name}
                          </button>
                        );
                      })
                    )}
                  </div>
                )}
              </>
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
