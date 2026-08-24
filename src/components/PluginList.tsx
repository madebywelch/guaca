import { useCallback, useEffect, useState } from "react";

import { api, openExternal } from "../lib/ipc";
import { hostOf, markFor } from "../lib/plugins";
import {
  type AccountConnection,
  type AgentCard,
  type AgentId,
  type CatalogKind,
  errorMessage,
  type GroupId,
  type Plugin,
  type PluginAccess,
  type PluginKind,
  type PluginOffer,
  type PluginToolCard,
} from "../lib/types";

interface Props {
  groupId: GroupId;
  /** The group's live agents, in rail order, for the ones a plugin is for. */
  crew: AgentCard[];
}

/**
 * The two answers to who may use a plugin, and to who may call one of its tools.
 *
 * Two buttons rather than a list of agents with an "everyone" entry at the top:
 * everyone is a standing answer that covers whoever is hired next week, and a
 * tick beside today's names cannot say that.
 *
 * One list for both levels, because it is one question asked about two things.
 * `short` is what a tool row draws and `label` is what the plugin row draws and
 * what both announce: a tool list is forty rows deep and two buttons reading
 * "Only chosen agents" on each of them crowds out the name the row is about,
 * while a reader who cannot see the row needs the whole sentence either way.
 */
const ACCESS_MODES = [
  { value: "everyone", label: "Every agent", short: "Everyone" },
  { value: "chosen", label: "Only chosen agents", short: "Chosen" },
] as const;

/** The chosen set with one agent added or taken out. */
function toggled(access: PluginAccess, agent: AgentId): AgentId[] {
  const current = access.mode === "chosen" ? access.agents : [];
  return current.includes(agent) ? current.filter((id) => id !== agent) : [...current, agent];
}

/** Whether one answer covers one agent. The webview's copy of `PluginAccess::allows`. */
function allows(access: PluginAccess, agent: AgentId): boolean {
  return access.mode === "everyone" || access.agents.includes(agent);
}

/** Who a connected plugin is offered to, in the operator's words. */
function offeredTo(access: PluginAccess, crew: AgentCard[]): string {
  if (access.mode === "everyone") return "offered to every agent in this group";
  const named = crew.filter((agent) => access.agents.includes(agent.id));
  if (named.length === 0) return "offered to nobody: tick an agent, or nothing here can be called";
  return `offered to ${named.map((agent) => agent.name).join(", ")}`;
}

/**
 * How much of what a plugin publishes the crew may actually call.
 *
 * The count is of everything the server published, not of what is left on,
 * because that number is what the row below expands into. The two kinds of
 * narrowing are counted apart because they are different decisions: a tool
 * given to some of the crew is working, and a tool given to nobody is one
 * nothing in this group can call. Both are said out loud for the reason a
 * plugin narrowed to nobody is: a connected plugin the crew cannot use is
 * indistinguishable from a working one until something says so.
 */
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

/**
 * Who can call one tool, once both answers are applied.
 *
 * The intersection, not the tool's own list, because the two questions compose:
 * an agent has to be on the plugin to spend its sign-in and on the tool to do
 * this particular thing with it. A row that read the tool's list alone would
 * name an agent that would be refused, which is the one thing a permission
 * panel must not do.
 *
 * A name ticked here that the plugin does not reach is kept rather than
 * dropped, and said out loud instead. The two controls are set in either order,
 * and silently discarding the ticks made before the plugin was widened would
 * lose work the operator can see themselves doing.
 */
function callers(plugin: Plugin, tool: PluginToolCard, crew: AgentCard[]): string {
  if (tool.access.mode === "everyone") return offeredTo(plugin.access, crew);
  const named = crew.filter((agent) => allows(tool.access, agent.id));
  const reach = named.filter((agent) => allows(plugin.access, agent.id));
  if (reach.length === 0) {
    return named.length === 0
      ? "switched off: nobody in this group can call it"
      : `nobody can call it: ${named.map((agent) => agent.name).join(", ")} ${
          named.length === 1 ? "is" : "are"
        } not on this plugin`;
  }
  const short = reach.map((agent) => agent.name).join(", ");
  const lost = named.filter((agent) => !allows(plugin.access, agent.id));
  if (lost.length === 0) return `called by ${short}`;
  return `called by ${short}; ${lost.map((agent) => agent.name).join(", ")} ${
    lost.length === 1 ? "is" : "are"
  } not on this plugin`;
}

/**
 * One line in the panel: a server, whether or not this crew has it.
 *
 * Assembled so the six on offer and the ones the operator added draw through
 * one piece of code. They differ in exactly two places — where the name and
 * address come from, and whether anybody vouched for the server — and every
 * other decision on the row is the same question about the same thing. Two
 * loops would be two places for "who can use it" to drift.
 */
interface Row {
  /** The name, which is also the prefix its tools are called by. */
  kind: PluginKind;
  name: string;
  endpoint: string;
  /** Only on a server Guaca ships: what it is for, and where to read about it. */
  offer?: PluginOffer;
  custom: boolean;
  held?: Plugin;
}

/**
 * Every server on this panel, offers first.
 *
 * The catalog's order is the backend's and is drawn as it arrives: a list the
 * webview sorted would be a second opinion about which servers exist. What the
 * operator added comes after it, in the order the store returns, which is by
 * name — the six are a starting point and the crew's own are the additions to
 * it, so that is the order somebody reads them in.
 */
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

/**
 * The plugins a crew has, and the ones it can have.
 *
 * A plugin is a server the operator signs in to once, on behalf of the whole
 * group, after which the agents they chose can call that server's tools.
 * Nothing is pasted and nothing is put on a machine: the call is made by Guaca
 * with the group's own sign-in on it, so the agent never holds a token and has
 * nothing to leak.
 *
 * Signing in, handing it out, and handing out one tool of it are three
 * decisions, and the second and third are on this row. Every agent is the
 * default and the usual answer for both; the crew's Stripe account is why the
 * plugin has another, and `create_refund` sitting beside `list_charges` is why
 * a tool does. The two compose, which is what lets one crew put the agent that
 * triages an inbox beside the agent that answers it, on one sign-in, with
 * different halves of it each.
 *
 * The third question is asked only where it was answered. A tool nobody has
 * touched draws two buttons and no sentence: forty rows each repeating the
 * default is the default said forty times, and it buries the one row that is
 * not it.
 *
 * Each change is written the moment it is made, like connecting and
 * disconnecting above it: there is no Save on this panel, so a draft nobody
 * submitted would be a permission the operator thinks they granted.
 *
 * The six at the top are a catalog rather than a limit, and the difference
 * matters. Each is on the list because somebody checked that it publishes its
 * own tools, acts on the operator's account and lets an application register
 * itself, and what that buys is a name, a sentence and a working sign-in behind
 * one click. Adding a server is the same mechanism with none of that done: the
 * operator supplies the two things the catalog was supplying, and everything
 * after that — the sign-in, the tool list, who may spend it, which tools are
 * whose — is the code above, unchanged. The row says nobody vouched for it,
 * because that is the whole of the difference.
 *
 * What is on the list comes from Rust, in order, and is drawn as it arrives: a
 * catalog the webview sorted or filtered would be a second opinion about
 * which servers exist, and the runtime is the one that dials them. The same
 * goes for a name: what an operator typed and what their agents will call it
 * are not always the same string, and the row draws what came back rather than
 * a second copy of the rule that turned one into the other.
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
  // Which plugins have their tool list open. Shut by default and not
  // remembered anywhere: a crew with three plugins connected has sixty rows
  // between the operator and the button they came here for, and the counts on
  // the line above are what says whether opening one is worth it.
  const [opened, setOpened] = useState<string[]>([]);
  // Which identities the operator has authorized at their Guaca account. Only
  // an account-backed plugin has any, and an install with no account has none,
  // which is why a failure here is swallowed rather than surfaced: it means
  // there is nothing to choose between, not that the panel is broken.
  const [connections, setConnections] = useState<AccountConnection[]>([]);
  // Which Guaca account those came from. Named on screen because an operator
  // can have more than one, and the failure that costs the most time is a
  // machine signed in to the account that does not hold the grant they are
  // looking at in a browser. Nothing else can tell them apart.
  const [accountEmail, setAccountEmail] = useState<string>("");
  // Which identity the operator picked for a plugin that is not connected yet,
  // keyed by kind. Only ever holds a pre-connect choice: once connected, the
  // stored row is what the picker reads, because that is what a turn will use.
  const [picked, setPicked] = useState<Record<string, string>>({});
  // The server being added, and the one being readdressed. Both are drafts and
  // neither is submitted on a keystroke, which is the one place on this panel
  // that is true: everything else here is a decision about a plugin that
  // already exists, and a half-typed URL is not a decision at all.
  const [draft, setDraft] = useState({ name: "", url: "", key: "" });
  const [adding, setAdding] = useState(false);
  // Keyed by plugin id, and absent until the operator opens the control: an
  // address box standing open under every added server is an invitation to
  // change something nobody came here to change.
  const [moving, setMoving] = useState<Record<string, { url: string; key: string }>>({});

  const load = useCallback(async () => {
    try {
      const [catalog, held] = await Promise.all([api.pluginCatalog(), api.groupPlugins(groupId)]);
      setOffers(catalog);
      setConnected(held);
      setError(null);
      // Separately, and deliberately not awaited with the two above: this one
      // goes over the network to guaca.bot, and a slow or absent service must
      // not keep the plugin list from drawing.
      void api
        .accountConnectors()
        .then((account) => {
          setConnections(account.connections);
          setAccountEmail(account.email);
        })
        .catch(() => {
          setConnections([]);
          setAccountEmail("");
        });
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
      {rows(offers, connected).map((row) => {
        const { kind, held, offer } = row;
        const brand = markFor(kind);
        const working = busy === kind;
        const chosen = held?.access.mode === "chosen" ? held.access.agents : [];
        // Two Google accounts are two grants, and a crew uses one of them.
        const mine = connections.filter((connection) => connection.provider === kind);
        // What this row is bound to, or about to be. A connected plugin reads
        // its stored row, because that is what a turn actually uses; one that
        // is not connected reads whatever the operator picked, defaulting to
        // the first.
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
              {/* The host, not the whole URL: what matters before authorizing
                  is which company is about to be handed the operator's
                  account, and the path is noise beside that. */}
              <span className="access__where">{hostOf(row.endpoint)}</span>

              {held ? (
                <button
                  type="button"
                  className="btn btn--small btn--ghost"
                  disabled={busy !== null}
                  onClick={() => void run(kind, () => api.disconnectPlugin(held.id))}
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
                      void run(kind, () =>
                        // What the picker above says, which defaults to the
                        // first identity. With none, this connects against the
                        // account's default and the refusal from Rust is what
                        // says an account is needed.
                        api.connectPlugin(groupId, offer.kind, using),
                      )
                    }
                  >
                    {working ? "Waiting for your browser…" : "Connect"}
                  </button>
                )
              )}
            </div>

            {/* Which of the account's identities this row uses.
                Shown whether or not the plugin is connected, because choosing
                is part of connecting: taking the first silently is how a crew
                ends up on the wrong mailbox with nothing on screen saying so.
                Only for a plugin whose credential is the account; the others
                sign in per group and their grant names its own identity. */}
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
                    onClick={() => void openExternal("https://guaca.bot/app")}
                  >
                    Authorize a {row.name} account
                  </button>
                </div>
              ) : mine.length === 1 ? (
                // One authorized account is not a decision, so this says which
                // it is rather than offering a control that cannot do anything.
                // It is still said out loud: a crew acting as somebody's mail
                // should never leave you guessing whose.
                <p className="field__hint">
                  {/* Tense matters. A group that has not connected this plugin
                      is not acting as anybody, and saying it is reads as a
                      grant that exists and does not. */}
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
                        // Nothing to write yet: remembered until Connect.
                        setPicked((was) => ({ ...was, [kind]: chosen }));
                        return;
                      }
                      // Connected, so this moves the crew. Its own command
                      // rather than a reconnect, which would replace the row
                      // and lose the per-tool switches.
                      void run(kind, () =>
                        api.setPluginConnection(groupId, offer.kind as CatalogKind, chosen),
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
                    Which {row.name} account this crew acts as
                    {accountEmail ? `, from your Guaca account ${accountEmail}` : ""}.{" "}
                    {held
                      ? "Its tools are re-read when you change it, because two accounts do not always authorize the same things."
                      : "Pick before connecting; you can change it afterward."}
                  </span>
                </label>
              ))}

            {/* Where a server the operator added actually is, and how to move
                it. Its own control rather than Disconnect and Add again, for
                the reason `setPluginConnection` is its own: those are different
                acts, and reconnecting replaces the row and loses the per-tool
                switches. A local server changing port is the common case and
                should not cost the operator their permissions. */}
            {held?.custom &&
              (moving[held.id] ? (
                <>
                  <label className="field">
                    <span className="field__label">Address</span>
                    <input
                      className="input"
                      value={moving[held.id]?.url ?? ""}
                      disabled={busy !== null}
                      onChange={(event) =>
                        setMoving((was) => ({
                          ...was,
                          [held.id]: { url: event.target.value, key: was[held.id]?.key ?? "" },
                        }))
                      }
                    />
                  </label>
                  <label className="field">
                    <span className="field__label">Key (optional)</span>
                    <input
                      className="input"
                      type="password"
                      value={moving[held.id]?.key ?? ""}
                      disabled={busy !== null}
                      onChange={(event) =>
                        setMoving((was) => ({
                          ...was,
                          [held.id]: { url: was[held.id]?.url ?? "", key: event.target.value },
                        }))
                      }
                    />
                    {/* Empty means "ask the server", not "keep what you had".
                        The stored key cannot be read back, so a box that
                        implied otherwise would be a box that silently kept a
                        key the operator meant to replace. */}
                    <span className="field__hint">
                      Paste one to replace the key this server has. Leave it empty and Guaca asks
                      the server what it wants, the way it does for a new one.
                    </span>
                  </label>
                  <div className="choices">
                    <button
                      type="button"
                      className="btn btn--small btn--primary"
                      disabled={busy !== null || !moving[held.id]?.url.trim()}
                      onClick={() =>
                        void run(`${kind}-address`, async () => {
                          const change = moving[held.id];
                          await api.readdressPlugin(
                            groupId,
                            held.id,
                            change?.url ?? "",
                            change?.key || undefined,
                          );
                          setMoving(({ [held.id]: _gone, ...rest }) => rest);
                        })
                      }
                    >
                      {busy === `${kind}-address` ? "Reconnecting…" : "Save and reconnect"}
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
                </>
              ) : (
                <p className="field__hint">
                  You added this one, so nobody has checked it. It gets everything a server on the
                  list above gets. <code>{held.endpoint}</code>{" "}
                  <button
                    type="button"
                    className="toolset__more"
                    disabled={busy !== null}
                    onClick={() =>
                      setMoving((was) => ({ ...was, [held.id]: { url: held.endpoint, key: "" } }))
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
                  {/* Blank for an account-backed plugin: whose account it is
                      is the Account line above, and a server's own name is not
                      an account. */}
                  {held.account && ` Signed in as ${held.account}.`}
                  {!held.signedIn &&
                    " This server asked for no sign-in, so nothing was authorized."}
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
                        void run(`${kind}-access`, () =>
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
                              void run(`${kind}-${agent.id}`, () =>
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

                {/* The second axis, and a different question from the first.
                    Who may spend the sign-in is about the account; which tools
                    may be spent is about the capability. A server does not
                    publish one kind of thing: Stripe lists the call that reads
                    an invoice beside the one that refunds it, and AgentMail
                    lists reading a thread beside sending as the operator. The
                    two answers compose, which is what lets one crew put the
                    agent that triages an inbox beside the agent that answers
                    it, on one sign-in, with different halves of it each. */}
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
                          {/* The vendor's own sentence, and the reason this is
                              a list rather than a row of names: `execute` and
                              `create_refund` are not decisions anybody can make
                              off the name alone. */}
                          {tool.description && (
                            <span className="toolset__blurb">{tool.description}</span>
                          )}
                          {/* Only for a tool somebody has decided about. Forty
                              rows each saying "offered to every agent in this
                              group" is the default repeated forty times, and it
                              buries the two rows that are not the default. */}
                          {tool.access.mode === "chosen" && (
                            <span className="toolset__who">{callers(held, tool, crew)}</span>
                          )}
                        </div>
                        <div className="choices">
                          {/* The same two answers the plugin above takes, and
                              the same two buttons, because it is the same
                              question about a smaller thing. "Only chosen" with
                              nothing ticked is the tool switched off for the
                              crew, which is still one click away and still what
                              the line above says out loud. */}
                          {ACCESS_MODES.map((mode) => (
                            <button
                              key={mode.value}
                              type="button"
                              className="choice choice--tight"
                              // Named for a reader who cannot see which row they
                              // are on. Forty buttons all called "Every agent"
                              // is a list only usable with a mouse.
                              aria-label={`${mode.label}: ${tool.name}`}
                              aria-pressed={tool.access.mode === mode.value}
                              disabled={busy !== null}
                              onClick={() => {
                                // Narrowing starts empty for the reason the
                                // plugin's does: the button whose purpose is to
                                // take something away must not be a click that
                                // changes nothing.
                                if (tool.access.mode === mode.value) return;
                                void run(`${kind}-${tool.name}`, () =>
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
                                  void run(`${kind}-${tool.name}-${agent.id}`, () =>
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
              )
            )}
          </div>
        );
      })}

      {/* Adding one. Below the six rather than beside them, because the order
          says what it is: the catalog is where to look first, and this is what
          to do when the server you want is not on it. */}
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
              {/* What the name is *for*, rather than the rules it has to obey.
                  The rules live in Rust, which normalizes what was typed, and a
                  second copy of them here would be a second place for them to
                  be wrong — so the row shows the name that came back instead of
                  predicting it. */}
              <span className="field__hint">
                {/* Not one of the six above, which would read as an instruction
                    to name it after a server that is already on the list. */}
                Your agents call this server's tools by this name: a server named
                <code> vault </code> offers <code>vault__read_secret</code>. Spaces and punctuation
                become underscores.
              </span>
            </label>
            <label className="field">
              <span className="field__label">Address</span>
              <input
                className="input"
                value={draft.url}
                placeholder="https://example.com/mcp"
                disabled={busy !== null}
                onChange={(event) => setDraft((was) => ({ ...was, url: event.target.value }))}
              />
              <span className="field__hint">
                The URL its MCP endpoint answers on. HTTPS, unless it is on this machine.
              </span>
            </label>
            <label className="field">
              <span className="field__label">Key (optional)</span>
              <input
                className="input"
                type="password"
                value={draft.key}
                disabled={busy !== null}
                onChange={(event) => setDraft((was) => ({ ...was, key: event.target.value }))}
              />
              {/* Both halves are worth saying. What it is for, because a server
                  that signs in properly needs nothing here and pasting a key
                  into a box marked optional is otherwise a guess; and where it
                  goes, because that is the whole promise a plugin makes. */}
              <span className="field__hint">
                For a server that wants a bearer token and has no sign-in of its own. Leave it empty
                and Guaca asks the server what it wants. Either way the key stays here: it never
                reaches a model, a transcript or an agent's machine.
              </span>
            </label>
            <div className="choices">
              <button
                type="button"
                className="btn btn--small btn--primary"
                disabled={busy !== null || !draft.name.trim() || !draft.url.trim()}
                onClick={() =>
                  void run("add", async () => {
                    await api.addPlugin(groupId, draft.name, draft.url, draft.key || undefined);
                    // Cleared only once it worked. A refused address the
                    // operator has to retype is a refusal that costs more than
                    // the mistake did.
                    setDraft({ name: "", url: "", key: "" });
                    setAdding(false);
                  })
                }
              >
                {busy === "add" ? "Connecting…" : "Add and connect"}
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
              list. Nobody has checked it, and it gets exactly what the six above get — so add one
              you would give the account to.
            </p>
          </>
        )}
      </div>

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
