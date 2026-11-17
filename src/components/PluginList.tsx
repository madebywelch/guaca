import { useCallback, useEffect, useReducer, useRef, useState } from "react";

import { api, openExternal } from "../lib/ipc";
import { hostOf, markFor, refusalWords, reportLine, STDIO } from "../lib/plugins";
import { useStore } from "../lib/store";
import {
  type AccountConnection,
  type AgentCard,
  type AgentId,
  type CatalogKind,
  errorMessage,
  type GroupId,
  type HeaderPair,
  type Plugin,
  type PluginAccess,
  type PluginKind,
  type PluginOffer,
  type PluginToolCard,
  type ServerReport,
} from "../lib/types";

interface Props {
  groupId: GroupId;
  crew: AgentCard[];
}

type Failure = ReturnType<typeof refusalWords>;

type PluginListState =
  | { state: "loading" }
  | { state: "failed" }
  | { state: "ready"; offers: PluginOffer[]; connected: Plugin[] };

type PluginListAction =
  | { type: "loaded"; offers: PluginOffer[]; connected: Plugin[] }
  | { type: "failed" };

export function pluginListReducer(
  _state: PluginListState,
  action: PluginListAction,
): PluginListState {
  switch (action.type) {
    case "loaded":
      return { state: "ready", offers: action.offers, connected: action.connected };
    case "failed":
      return { state: "failed" };
  }
}

const ACCESS_MODES = [
  { value: "everyone", label: "Every agent", short: "All agents" },
  { value: "chosen", label: "Only chosen agents", short: "Chosen" },
] as const;

interface HeaderDraft extends HeaderPair {
  id: number;
}

let headerRows = 0;
function blankHeader(name = "", value = ""): HeaderDraft {
  headerRows += 1;
  return { id: headerRows, name, value };
}

function sent(rows: HeaderDraft[]): HeaderPair[] {
  return rows.map(({ name, value }) => ({ name, value }));
}

function HeaderFields({
  rows,
  onChange,
  disabled,
  env = false,
}: {
  rows: HeaderDraft[];
  onChange: (rows: HeaderDraft[]) => void;
  disabled: boolean;
  env?: boolean;
}) {
  const noun = env ? "Variable" : "Header";
  const edit = (id: number, part: Partial<HeaderPair>) =>
    onChange(rows.map((was) => (was.id === id ? { ...was, ...part } : was)));

  return (
    <div className="field">
      <span className="field__label">{env ? "Environment (optional)" : "Headers (optional)"}</span>
      {rows.map((row, at) => (
        <div className="choices" key={row.id}>
          <input
            className="input"
            placeholder={env ? "GITHUB_TOKEN" : "X-API-Key"}
            aria-label={`${noun} ${at + 1} name`}
            value={row.name}
            disabled={disabled}
            onChange={(event) => edit(row.id, { name: event.target.value })}
          />
          <input
            className="input"
            type="password"
            placeholder="value"
            aria-label={`${noun} ${at + 1} value`}
            value={row.value}
            disabled={disabled}
            onChange={(event) => edit(row.id, { value: event.target.value })}
          />
          <button
            type="button"
            className="btn btn--small btn--ghost"
            aria-label={`Remove ${noun.toLowerCase()} ${at + 1}`}
            disabled={disabled}
            onClick={() => onChange(rows.filter((was) => was.id !== row.id))}
          >
            Remove
          </button>
        </div>
      ))}
      <p className="field__hint">
        {env
          ? "Set in the program's environment, which starts empty apart from what it needs to find its own files: usually its token, under the name its instructions give. A value never reaches a model, a transcript or an agent's machine, and never comes back to this panel."
          : "Sent on every request to this server, whatever else authorizes it: an API key the server reads from a header it named, or the pair a gate in front of it wants. They stay here: a value never reaches a model, a transcript or an agent's machine, and never comes back to this panel."}
      </p>
      <button
        type="button"
        className="toolset__more"
        disabled={disabled}
        onClick={() => onChange([...rows, blankHeader()])}
      >
        {env ? "Add a variable" : "Add a header"}
      </button>
    </div>
  );
}

function toggled(access: PluginAccess, agent: AgentId): AgentId[] {
  const current = access.mode === "chosen" ? access.agents : [];
  return current.includes(agent) ? current.filter((id) => id !== agent) : [...current, agent];
}

function allows(access: PluginAccess, agent: AgentId): boolean {
  return access.mode === "everyone" || access.agents.includes(agent);
}

function offeredTo(access: PluginAccess, crew: AgentCard[]): string {
  if (access.mode === "everyone") return "offered to every agent in this group";
  const named = crew.filter((agent) => access.agents.includes(agent.id));
  if (named.length === 0) return "offered to nobody: tick an agent, or nothing here can be called";
  return `offered to ${named.map((agent) => agent.name).join(", ")}`;
}

function offering(tools: PluginToolCard[]): string {
  const narrowed = tools.filter((tool) => tool.access.mode === "chosen");
  const off = narrowed.filter(
    (tool) => tool.access.mode === "chosen" && tool.access.agents.length === 0,
  ).length;
  const some = narrowed.length - off;
  const count = `${tools.length} tool${tools.length === 1 ? "" : "s"}`;
  if (narrowed.length === 0) return count;
  if (off === tools.length) return `${count}, all switched off: none of them can be called`;
  const said = [
    some > 0 ? `${some} for chosen agents` : null,
    off > 0 ? `${off} switched off` : null,
  ].filter(Boolean);
  return `${count}, ${said.join(" and ")}`;
}

function callers(plugin: Plugin, tool: PluginToolCard, crew: AgentCard[]): string {
  if (tool.access.mode === "everyone") return offeredTo(plugin.access, crew);
  const named = crew.filter((agent) => allows(tool.access, agent.id));
  const reach = named.filter((agent) => allows(plugin.access, agent.id));
  if (reach.length === 0) {
    return named.length === 0
      ? "switched off: nobody in this group can call it"
      : `nobody can call it: ${named.map((agent) => agent.name).join(", ")} ${
          named.length === 1 ? "is" : "are"
        } not on this connector`;
  }
  const short = reach.map((agent) => agent.name).join(", ");
  const lost = named.filter((agent) => !allows(plugin.access, agent.id));
  if (lost.length === 0) return `called by ${short}`;
  return `called by ${short}; ${lost.map((agent) => agent.name).join(", ")} ${
    lost.length === 1 ? "is" : "are"
  } not on this connector`;
}

interface Row {
  kind: PluginKind;
  name: string;
  endpoint: string;
  offer?: PluginOffer;
  custom: boolean;
  held?: Plugin;
}

function rows(offers: PluginOffer[], connected: Plugin[]): Row[] {
  const shipped: Row[] = offers.map((offer) => ({
    kind: offer.kind,
    name: offer.name,
    endpoint: offer.endpoint,
    offer,
    custom: false,
    held: connected.find((plugin) => plugin.kind === offer.kind),
  }));
  const added: Row[] = connected
    .filter((plugin) => plugin.custom)
    .map((plugin) => ({
      kind: plugin.kind,
      name: plugin.name,
      endpoint: plugin.endpoint,
      custom: true,
      held: plugin,
    }));
  return [...shipped, ...added];
}

export function PluginList({ groupId, crew }: Props) {
  return <GroupPluginList key={groupId} groupId={groupId} crew={crew} />;
}

function GroupPluginList({ groupId, crew }: Props) {
  const [list, dispatchList] = useReducer(pluginListReducer, { state: "loading" });
  const generation = useRef(0);
  const [busy, setBusy] = useState<{ key: string; says: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [failed, setFailed] = useState<Record<string, Failure | undefined>>({});
  const [opened, setOpened] = useState<string[]>([]);
  const [connections, setConnections] = useState<AccountConnection[]>([]);
  const [accountEmail, setAccountEmail] = useState<string>("");
  const [picked, setPicked] = useState<Record<string, string>>({});
  const [draft, setDraft] = useState<{
    name: string;
    url: string;
    key: string;
    headers: HeaderDraft[];
    program: boolean;
    command: string;
  }>({ name: "", url: "", key: "", headers: [], program: false, command: "" });
  const address = draft.program ? `${STDIO}${draft.command.trim()}` : draft.url;
  const addressed = draft.program ? draft.command.trim() !== "" : draft.url.trim() !== "";
  const [adding, setAdding] = useState(false);
  const [moving, setMoving] = useState<
    Record<string, { url: string; key: string; headers: HeaderDraft[] | null }>
  >({});
  const [tested, setTested] = useState<Record<string, ServerReport>>({});

  const load = useCallback(async () => {
    const request = ++generation.current;
    try {
      const [catalog, held] = await Promise.all([api.pluginCatalog(), api.groupPlugins(groupId)]);
      if (request !== generation.current) return;
      dispatchList({ type: "loaded", offers: catalog, connected: held });
      setError(null);
      void api
        .accountConnectors()
        .then((account) => {
          if (request !== generation.current) return;
          setConnections(account.connections);
          setAccountEmail(account.email);
        })
        .catch(() => {
          if (request !== generation.current) return;
          setConnections([]);
          setAccountEmail("");
        });
    } catch (caught) {
      if (request !== generation.current) return;
      setError(errorMessage(caught));
      dispatchList({ type: "failed" });
    }
  }, [groupId]);

  useEffect(() => {
    void load();
    return () => {
      generation.current += 1;
    };
  }, [load]);

  const run = async (key: string, says: string, action: () => Promise<unknown>, at?: string) => {
    setBusy({ key, says });
    setError(null);
    if (at) setFailed((was) => ({ ...was, [at]: undefined }));
    try {
      await action();
      await load();
    } catch (caught) {
      if (at) setFailed((was) => ({ ...was, [at]: refusalWords(caught) }));
      else setError(errorMessage(caught));
    } finally {
      setBusy(null);
      useStore.getState().setHandoff(null);
    }
  };

  const test = async (key: string, says: string, dial: () => Promise<ServerReport>, at: string) => {
    setBusy({ key, says });
    setError(null);
    setFailed((was) => ({ ...was, [at]: undefined }));
    try {
      const report = await dial();
      setTested((was) => ({ ...was, [key]: report }));
    } catch (caught) {
      setTested(({ [key]: _gone, ...rest }) => rest);
      setFailed((was) => ({ ...was, [at]: refusalWords(caught) }));
    } finally {
      setBusy(null);
    }
  };

  const forget = (report: string, failure: string) => {
    setTested(({ [report]: _gone, ...rest }) => rest);
    setFailed((was) => ({ ...was, [failure]: undefined }));
  };

  if (list.state === "loading") return <p className="field__hint">Loading connectors…</p>;

  const offers = list.state === "ready" ? list.offers : [];
  const connected = list.state === "ready" ? list.connected : [];

  return (
    <div className="access">
      {rows(offers, connected).map((row) => {
        const { kind, held, offer } = row;
        const brand = markFor(kind);
        const working = busy?.key === kind;
        const chosen = held?.access.mode === "chosen" ? held.access.agents : [];
        const mine = connections.filter((connection) => connection.provider === kind);
        const using = held ? held.connection || mine[0]?.id : (picked[kind] ?? mine[0]?.id);

        return (
          <div className="access__item" key={kind}>
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
              <strong className="access__name">{row.name}</strong>
              <span className="access__where">{hostOf(row.endpoint)}</span>

              {held ? (
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  disabled={busy !== null}
                  onClick={() =>
                    void run(
                      kind,
                      "Disconnecting… Its tools leave this group as soon as the host answers.",
                      () => api.disconnectPlugin(held.id),
                    )
                  }
                >
                  {working ? "Disconnecting…" : "Disconnect"}
                </button>
              ) : (
                offer && (
                  <button
                    type="button"
                    className="btn btn--small btn--primary"
                    disabled={busy !== null}
                    onClick={() =>
                      void run(
                        kind,
                        `Connecting ${row.name}… If it asks for a sign-in, finish in the browser window that just opened. Guaca is waiting, and gives up after five minutes.`,
                        () => api.connectPlugin(groupId, offer.kind, using),
                      )
                    }
                  >
                    {working ? "Waiting for your browser…" : "Connect"}
                  </button>
                )
              )}
            </div>

            {offer?.accountBacked &&
              (mine.length === 0 ? (
                <div className="access__empty">
                  <p className="field__hint">
                    No {row.name} account is authorized on
                    {accountEmail ? ` your Guaca account, ${accountEmail}` : " your Guaca account"}.
                    Authorize one there, then connect it here.
                  </p>
                  <button
                    type="button"
                    className="btn btn--ghost btn--small"
                    onClick={() => {
                      void api
                        .accountStatus()
                        .then(async (account) => {
                          if (account.broker) await api.signInAccount();
                          else await openExternal(`${account.origin}/app`);
                        })
                        .catch((caught) => setError(errorMessage(caught)));
                    }}
                  >
                    Authorize a {row.name} account
                  </button>
                </div>
              ) : mine.length === 1 ? (
                <p className="field__hint">
                  {held ? "Acting as " : "Will connect as "}
                  {mine.find((connection) => connection.id === using)?.label ?? mine[0]?.label}
                  {accountEmail ? `, from your Guaca account ${accountEmail}.` : "."}
                </p>
              ) : (
                <label className="field">
                  <span className="field__label">Account</span>
                  <select
                    className="input"
                    value={using ?? ""}
                    disabled={busy !== null}
                    onChange={(event) => {
                      const chosen = event.target.value;
                      if (!held) {
                        setPicked((was) => ({ ...was, [kind]: chosen }));
                        return;
                      }
                      void run(
                        `${kind}-account`,
                        "Switching the account… Its tools are re-read with the new one.",
                        () => api.setPluginConnection(groupId, offer.kind as CatalogKind, chosen),
                      );
                    }}
                  >
                    {mine.map((connection) => (
                      <option key={connection.id} value={connection.id}>
                        {connection.label}
                      </option>
                    ))}
                  </select>
                  <span className="field__hint">
                    Which {row.name} account this group acts as
                    {accountEmail ? `, from your Guaca account ${accountEmail}` : ""}.{" "}
                    {held
                      ? "Its tools are re-read when you change it, because two accounts do not always authorize the same things."
                      : "Pick before connecting; you can change it afterward."}
                  </span>
                </label>
              ))}

            {held?.custom &&
              (moving[held.id] ? (
                <>
                  <label className="field">
                    <span className="field__label">Address</span>
                    <input
                      className="input"
                      value={moving[held.id]?.url ?? ""}
                      disabled={busy !== null}
                      onChange={(event) => {
                        setMoving((was) => ({
                          ...was,
                          [held.id]: { ...was[held.id]!, url: event.target.value },
                        }));
                        forget(`${kind}-address-test`, `address:${held.id}`);
                      }}
                    />
                  </label>
                  <label className="field">
                    <span className="field__label">Key (optional)</span>
                    <input
                      className="input"
                      type="password"
                      value={moving[held.id]?.key ?? ""}
                      disabled={busy !== null}
                      onChange={(event) => {
                        setMoving((was) => ({
                          ...was,
                          [held.id]: { ...was[held.id]!, key: event.target.value },
                        }));
                        forget(`${kind}-address-test`, `address:${held.id}`);
                      }}
                    />
                    <span className="field__hint">
                      Paste one to replace the key this server has. Leave it empty and Guaca asks
                      the server what it wants, the way it does for a new one.
                    </span>
                  </label>
                  {moving[held.id]?.headers ? (
                    <HeaderFields
                      rows={moving[held.id]?.headers ?? []}
                      disabled={busy !== null}
                      onChange={(rows) => {
                        setMoving((was) => ({
                          ...was,
                          [held.id]: { ...was[held.id]!, headers: rows },
                        }));
                        forget(`${kind}-address-test`, `address:${held.id}`);
                      }}
                    />
                  ) : (
                    <p className="field__hint">
                      {held.headers.length === 0
                        ? "No headers. "
                        : `Sends ${held.headers.join(", ")}. `}
                      <button
                        type="button"
                        className="toolset__more"
                        disabled={busy !== null}
                        onClick={() => {
                          setMoving((was) => ({
                            ...was,
                            [held.id]: {
                              ...was[held.id]!,
                              headers: held.headers.map((name) => blankHeader(name)),
                            },
                          }));
                          forget(`${kind}-address-test`, `address:${held.id}`);
                        }}
                      >
                        Replace headers
                      </button>
                    </p>
                  )}
                  {failed[`address:${held.id}`] && (
                    <ConnectorFailure failure={failed[`address:${held.id}`]!} />
                  )}
                  <div className="choices">
                    <button
                      type="button"
                      className="btn btn--small btn--primary"
                      disabled={busy !== null || !moving[held.id]?.url.trim()}
                      onClick={() =>
                        void run(
                          `${kind}-address`,
                          "Saving the new details and reconnecting… If it asks for a sign-in, finish in the browser window that just opened. Guaca is waiting, and gives up after five minutes.",
                          async () => {
                            const change = moving[held.id];
                            await api.readdressPlugin(
                              groupId,
                              held.id,
                              change?.url ?? "",
                              change?.key || undefined,
                              change?.headers ? sent(change.headers) : undefined,
                            );
                            setMoving(({ [held.id]: _gone, ...rest }) => rest);
                            forget(`${kind}-address-test`, `address:${held.id}`);
                            setTested(({ [`${kind}-check`]: _gone, ...rest }) => rest);
                          },
                          `address:${held.id}`,
                        )
                      }
                    >
                      {busy?.key === `${kind}-address` ? "Reconnecting…" : "Save and reconnect"}
                    </button>
                    <button
                      type="button"
                      className="btn btn--small btn--ghost"
                      disabled={busy !== null || !moving[held.id]?.url.trim()}
                      onClick={() =>
                        void test(
                          `${kind}-address-test`,
                          "Testing the address… It dials the server and reads its tools, and connects nothing. It gives up if the server stops answering.",
                          () => {
                            const change = moving[held.id];
                            return api.probeServer(
                              change?.url ?? "",
                              change?.key || undefined,
                              change?.headers ? sent(change.headers) : undefined,
                            );
                          },
                          `address:${held.id}`,
                        )
                      }
                    >
                      {busy?.key === `${kind}-address-test` ? "Testing…" : "Test"}
                    </button>
                    <button
                      type="button"
                      className="btn btn--small btn--ghost"
                      disabled={busy !== null}
                      onClick={() => setMoving(({ [held.id]: _gone, ...rest }) => rest)}
                    >
                      Cancel
                    </button>
                  </div>
                  {tested[`${kind}-address-test`] && (
                    <p className="field__hint">{reportLine(tested[`${kind}-address-test`]!)}</p>
                  )}
                </>
              ) : (
                <p className="field__hint">
                  You added this one, so nobody has checked it. It gets everything a server on the
                  list above gets. <code>{held.endpoint}</code>
                  {held.headers.length > 0 && ` Sends ${held.headers.join(", ")}.`}{" "}
                  <button
                    type="button"
                    className="toolset__more"
                    disabled={busy !== null}
                    onClick={() =>
                      setMoving((was) => ({
                        ...was,
                        [held.id]: { url: held.endpoint, key: "", headers: null },
                      }))
                    }
                  >
                    Change address
                  </button>
                </p>
              ))}

            {held ? (
              <>
                <p className="field__hint">
                  {offering(held.tools)}, {offeredTo(held.access, crew)}.
                  {held.account && ` Signed in as ${held.account}.`}
                  {!held.signedIn &&
                    " This server asked for no sign-in, so nothing was authorized."}{" "}
                  <button
                    type="button"
                    className="toolset__more"
                    disabled={busy !== null}
                    onClick={() =>
                      void test(
                        `${kind}-check`,
                        "Testing the connection… It asks the server again, reusing this group's sign-in if there is one, and writes nothing. It gives up if the server stops answering.",
                        () => api.checkPlugin(held.id),
                        `check:${held.id}`,
                      )
                    }
                  >
                    {busy?.key === `${kind}-check` ? "Testing…" : "Test it"}
                  </button>
                </p>

                {tested[`${kind}-check`] && (
                  <p className="field__hint">{reportLine(tested[`${kind}-check`]!)}</p>
                )}
                {failed[`check:${held.id}`] && (
                  <ConnectorFailure failure={failed[`check:${held.id}`]!} />
                )}

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
                        if (held.access.mode === mode.value) return;
                        void run(`${kind}-access`, "Saving who can use it…", () =>
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
                            aria-pressed={has}
                            disabled={busy !== null}
                            onClick={() =>
                              void run(`${kind}-${agent.id}`, "Saving who can use it…", () =>
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

                {held.tools.length > 0 && (
                  <>
                    <span className="field__label">Which of its tools, and whose</span>
                    <button
                      type="button"
                      className="toolset__more"
                      aria-expanded={opened.includes(kind)}
                      onClick={() =>
                        setOpened((open) =>
                          open.includes(kind)
                            ? open.filter((open) => open !== kind)
                            : [...open, kind],
                        )
                      }
                    >
                      {opened.includes(kind) ? "Hide them" : `Show all ${held.tools.length}`}
                    </button>
                  </>
                )}

                {opened.includes(kind) && (
                  <ul className="toolset">
                    {held.tools.map((tool) => (
                      <li className="toolset__item" key={tool.name}>
                        <div className="toolset__text">
                          <code className="toolset__name">{tool.name}</code>
                          {tool.description && (
                            <span className="toolset__blurb">{tool.description}</span>
                          )}
                          {tool.access.mode === "chosen" && (
                            <span className="toolset__who">{callers(held, tool, crew)}</span>
                          )}
                        </div>
                        <div className="choices">
                          {ACCESS_MODES.map((mode) => (
                            <button
                              key={mode.value}
                              type="button"
                              className="choice choice--tight"
                              aria-label={`${mode.label}: ${tool.name}`}
                              aria-pressed={tool.access.mode === mode.value}
                              disabled={busy !== null}
                              onClick={() => {
                                if (tool.access.mode === mode.value) return;
                                void run(
                                  `${kind}-${tool.name}`,
                                  "Saving which tools are offered…",
                                  () =>
                                    api.setPluginTool(
                                      held.id,
                                      tool.name,
                                      mode.value === "everyone"
                                        ? { mode: "everyone" }
                                        : { mode: "chosen", agents: [] },
                                    ),
                                );
                              }}
                            >
                              {mode.short}
                            </button>
                          ))}
                        </div>

                        {tool.access.mode === "chosen" && crew.length > 0 && (
                          <div className="choices toolset__crew">
                            {crew.map((agent) => (
                              <button
                                key={agent.id}
                                type="button"
                                className="choice choice--tight"
                                aria-label={`${agent.name}: ${tool.name}`}
                                aria-pressed={allows(tool.access, agent.id)}
                                disabled={busy !== null}
                                onClick={() =>
                                  void run(
                                    `${kind}-${tool.name}-${agent.id}`,
                                    "Saving which tools are offered…",
                                    () =>
                                      api.setPluginTool(held.id, tool.name, {
                                        mode: "chosen",
                                        agents: toggled(tool.access, agent.id),
                                      }),
                                  )
                                }
                              >
                                {agent.name}
                              </button>
                            ))}
                          </div>
                        )}
                      </li>
                    ))}
                  </ul>
                )}
              </>
            ) : (
              offer && (
                <p className="field__hint">
                  {offer.blurb}{" "}
                  <a
                    href={offer.docs}
                    target="_blank"
                    rel="noreferrer"
                    className="access__docs"
                    onClick={(event) => {
                      event.preventDefault();
                      void openExternal(offer.docs);
                    }}
                  >
                    What this can do
                  </a>
                </p>
              )
            )}
          </div>
        );
      })}

      <div className="access__item">
        {adding ? (
          <>
            <span className="field__label">Add a server</span>
            <label className="field">
              <span className="field__label">Name</span>
              <input
                className="input"
                value={draft.name}
                placeholder="Home Assistant"
                disabled={busy !== null}
                onChange={(event) => setDraft((was) => ({ ...was, name: event.target.value }))}
              />
              <span className="field__hint">
                Your agents call this server's tools by this name. Spaces and punctuation become
                underscores.
              </span>
            </label>
            <fieldset className="choices" aria-label="How Guaca reaches it">
              <button
                type="button"
                className="choice"
                aria-pressed={!draft.program}
                disabled={busy !== null}
                onClick={() => {
                  setDraft((was) => ({ ...was, program: false, headers: [] }));
                  forget("add-test", "add");
                }}
              >
                At an address
              </button>
              <button
                type="button"
                className="choice"
                aria-pressed={draft.program}
                disabled={busy !== null}
                onClick={() => {
                  setDraft((was) => ({ ...was, program: true, headers: [], key: "" }));
                  forget("add-test", "add");
                }}
              >
                A program on the host
              </button>
            </fieldset>
            {draft.program ? (
              <label className="field">
                <span className="field__label">Command</span>
                <input
                  className="input input--mono"
                  value={draft.command}
                  placeholder="npx -y @modelcontextprotocol/server-github"
                  disabled={busy !== null}
                  onChange={(event) => {
                    setDraft((was) => ({ ...was, command: event.target.value }));
                    forget("add-test", "add");
                  }}
                />
                <span className="field__hint">
                  The program and its arguments, as you would type them in a terminal on the machine
                  this workspace runs on. It is started for each call and never given to a shell.
                </span>
              </label>
            ) : (
              <label className="field">
                <span className="field__label">Address</span>
                <input
                  className="input"
                  value={draft.url}
                  placeholder="https://example.com/mcp"
                  disabled={busy !== null}
                  onChange={(event) => {
                    setDraft((was) => ({ ...was, url: event.target.value }));
                    forget("add-test", "add");
                  }}
                />
                <span className="field__hint">
                  The URL its MCP endpoint answers on. HTTPS, unless it is on this machine.
                </span>
              </label>
            )}
            <label className="field" hidden={draft.program}>
              <span className="field__label">Key (optional)</span>
              <input
                className="input"
                type="password"
                value={draft.key}
                disabled={busy !== null}
                onChange={(event) => {
                  setDraft((was) => ({ ...was, key: event.target.value }));
                  forget("add-test", "add");
                }}
              />
              <span className="field__hint">
                For a server that wants a bearer token and has no sign-in of its own. Leave it empty
                and Guaca asks the server what it wants. Either way the key stays here: it never
                reaches a model, a transcript or an agent's machine.
              </span>
            </label>
            <HeaderFields
              rows={draft.headers}
              disabled={busy !== null}
              env={draft.program}
              onChange={(headers) => {
                setDraft((was) => ({ ...was, headers }));
                forget("add-test", "add");
              }}
            />
            {failed.add && <ConnectorFailure failure={failed.add} />}
            <div className="choices">
              <button
                type="button"
                className="btn btn--small btn--primary"
                disabled={busy !== null || !draft.name.trim() || !addressed}
                onClick={() =>
                  void run(
                    "add",
                    `Adding ${draft.name}… If it asks for a sign-in, finish in the browser window that just opened. Guaca is waiting, and gives up after five minutes.`,
                    async () => {
                      await api.addPlugin(
                        groupId,
                        draft.name,
                        address,
                        draft.key || undefined,
                        sent(draft.headers),
                      );
                      setDraft({
                        name: "",
                        url: "",
                        key: "",
                        headers: [],
                        program: false,
                        command: "",
                      });
                      setAdding(false);
                      forget("add-test", "add");
                    },
                    "add",
                  )
                }
              >
                {busy?.key === "add" ? "Connecting…" : "Add and connect"}
              </button>
              <button
                type="button"
                className="btn btn--small btn--ghost"
                disabled={busy !== null || !addressed}
                onClick={() =>
                  void test(
                    "add-test",
                    "Testing the address… It dials the server and reads its tools, and connects nothing. It gives up if the server stops answering.",
                    () => api.probeServer(address, draft.key || undefined, draft.headers),
                    "add",
                  )
                }
              >
                {busy?.key === "add-test" ? "Testing…" : "Test"}
              </button>
              <button
                type="button"
                className="btn btn--small btn--ghost"
                disabled={busy !== null}
                onClick={() => setAdding(false)}
              >
                Cancel
              </button>
            </div>
            {tested["add-test"] && <p className="field__hint">{reportLine(tested["add-test"]!)}</p>}
          </>
        ) : (
          <>
            <button
              type="button"
              className="btn btn--small btn--ghost"
              disabled={busy !== null}
              onClick={() => setAdding(true)}
            >
              Add a server
            </button>
            <p className="field__hint">
              Any MCP server: one you run, one your company runs, or a vendor that is not on this
              list. Nobody has checked it, and it gets exactly what the eight above get, so add one
              you would give the account to.
            </p>
          </>
        )}
      </div>

      {busy !== null && <p className="field__hint">{busy.says}</p>}

      {error && (
        <div className="banner banner--error" style={{ margin: "0.4rem 0 0" }}>
          <span>{error}</span>
        </div>
      )}
    </div>
  );
}

function ConnectorFailure({ failure }: { failure: Failure }) {
  return (
    <div className="connector-failure" role="alert">
      <p className="connector-failure__says">{failure.says}</p>
      {failure.detail && (
        <details className="connector-failure__detail">
          <summary>The raw error</summary>
          <pre>{failure.detail}</pre>
        </details>
      )}
    </div>
  );
}
