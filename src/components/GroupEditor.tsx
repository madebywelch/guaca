/**
 * A group's name, its wall, and the settings its agents run on.
 *
 * Sectioned like Settings, and for the same reason: a group now decides who
 * pays for its turns, which model answers them and how far a conversation may
 * run, which is more than one scroll can hold without the name and the delete
 * button ending up a page apart. Every piece of state lives in the shell, so
 * changing section cannot discard a half-typed endpoint.
 *
 * Everything except the name is an override, and blank means inherit. That is
 * why the placeholders show what the app would use instead of a value: an
 * operator has to be able to tell "this group uses the app model" apart from
 * "this group pins that exact model".
 */

import { useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { LIMITS } from "../lib/limits";
import { type Provider as Preset, planLabel, providerFor } from "../lib/providers";
import { useStore } from "../lib/store";
import {
  errorMessage,
  type Group,
  type GroupDraft,
  type GroupReset,
  type Provider,
  type SubscriptionStatus,
} from "../lib/types";
import { CredentialList } from "./CredentialList";
import { GroupActivity } from "./GroupActivity";
import { PluginList } from "./PluginList";
import { ProviderPresets, SubscriptionModel } from "./ProviderFields";
import { RepositoryList } from "./RepositoryList";

interface Props {
  /** Absent means create. */
  group?: Group;
  onClose: () => void;
}

const SECTIONS = ["general", "provider", "limits", "plugins", "repositories", "activity"] as const;

type Section = (typeof SECTIONS)[number];

const SECTION_LABELS: Record<Section, string> = {
  general: "General",
  provider: "Provider",
  limits: "Limits",
  plugins: "Plugins",
  repositories: "Repositories",
  activity: "Activity",
};

/**
 * The sections that are about something attached to a crew, so a crew has to
 * exist first. A sign-in, a credential and a linked directory all have to
 * belong to something, and there is no row to hang any of them on until the
 * group is created.
 */
const NEEDS_GROUP: readonly Section[] = ["plugins", "repositories", "activity"];

/** What a group says when it has no opinion about who pays. */
const INHERIT = "inherit";

/** A number an operator may leave blank, as it is typed and as it is stored. */
const asText = (value: number | null | undefined) => (value === null ? "" : String(value ?? ""));
const asNumber = (text: string) => (text.trim() ? Number(text) : null);

export function GroupEditor({ group, onClose }: Props) {
  const refreshAgents = useStore((s) => s.refreshAgents);
  const settings = useStore((s) => s.settings);
  const capabilities = useStore((s) => s.capabilities);
  const agents = useStore((s) => s.agents);

  const [section, setSection] = useState<Section>("general");
  const [name, setName] = useState(group?.name ?? "");
  const [provider, setProvider] = useState<Provider | typeof INHERIT>(
    group?.inference.provider ?? INHERIT,
  );
  const [baseUrl, setBaseUrl] = useState(group?.inference.baseUrl ?? "");
  const [model, setModel] = useState(group?.inference.defaultModel ?? "");
  const [subscriptionModel, setSubscriptionModel] = useState(
    group?.inference.subscriptionModel ?? "",
  );
  const [timeout, setTimeoutSecs] = useState(asText(group?.inference.requestTimeoutSecs));
  // Null until the operator types, because absent and blank are different
  // instructions: one keeps the stored key, the other clears it.
  const [apiKey, setApiKey] = useState<string | null>(null);
  const [limits, setLimits] = useState<Record<string, string>>(() =>
    Object.fromEntries(
      LIMITS.map((field) => [field.key, asText(group?.limits[field.key] ?? null)]),
    ),
  );

  const [status, setStatus] = useState<{ tone: "ok" | "error"; text: string } | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [confirmClear, setConfirmClear] = useState(false);
  const [cleared, setCleared] = useState<GroupReset | null>(null);
  const [subscription, setSubscription] = useState<SubscriptionStatus | null>(null);
  const nameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    nameRef.current?.focus();
    nameRef.current?.select();
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Read when the pane that shows it is opened, not on mount: a group's name is
  // what most edits here are, and a status nobody is looking at is a round trip
  // spent for nothing.
  useEffect(() => {
    if (section !== "provider" || subscription) return;
    let live = true;
    void api
      .subscriptionStatus()
      .then((value) => {
        if (live) setSubscription(value);
      })
      .catch(() => {
        if (live) setSubscription({ signedIn: false, email: "", plan: "", includesCodex: false });
      });
    return () => {
      live = false;
    };
  }, [section, subscription]);

  const draft = (): GroupDraft => ({
    name,
    inference: {
      provider: provider === INHERIT ? null : provider,
      baseUrl: baseUrl.trim() || null,
      defaultModel: model.trim() || null,
      subscriptionModel: subscriptionModel.trim() || null,
      requestTimeoutSecs: asNumber(timeout),
    },
    // Only sent once the operator has touched it. Sending the redacted hint
    // back would overwrite the real key with its own placeholder.
    ...(apiKey !== null ? { apiKey } : {}),
    limits: {
      maxHops: asNumber(limits.maxHops ?? ""),
      maxStepsPerRun: asNumber(limits.maxStepsPerRun ?? ""),
      maxFanoutPerCall: asNumber(limits.maxFanoutPerCall ?? ""),
      maxSendsPerPair: asNumber(limits.maxSendsPerPair ?? ""),
      maxToolRounds: asNumber(limits.maxToolRounds ?? ""),
    },
  });

  /**
   * Whether anything here is waiting to be saved.
   *
   * Read across every pane, because the shell holds all of their state: an
   * endpoint typed on Provider and left there while the operator signs a plugin
   * in is still unsaved, and a Save that went missing on the way past is how it
   * would get lost.
   *
   * What it buys is a foot that offers nothing on the two panes that stage
   * nothing. A plugin is signed in and a repository is linked at the moment the
   * operator does it, so a Save under either was a button offering to save work
   * that had already been saved, beside a Cancel implying it could be taken
   * back. A group that does not exist yet is always waiting: there is nothing
   * to compare it with, and Create is the only way out that leaves one behind.
   */
  const dirty =
    !group ||
    name !== group.name ||
    provider !== (group.inference.provider ?? INHERIT) ||
    baseUrl !== (group.inference.baseUrl ?? "") ||
    model !== (group.inference.defaultModel ?? "") ||
    subscriptionModel !== (group.inference.subscriptionModel ?? "") ||
    timeout !== asText(group.inference.requestTimeoutSecs) ||
    // Typed at all, including typed and then emptied, which is the one
    // instruction that puts a group back on the app's key.
    apiKey !== null ||
    LIMITS.some((field) => limits[field.key] !== asText(group.limits[field.key]));

  /** Live agents. Terminated ones are not in the count and are not deleted twice. */
  const crew = group?.agentCount ?? 0;

  /**
   * The crew itself, for the plugins panel: who a plugin can be narrowed to.
   *
   * Read from the rail's own list rather than fetched, so a rename or a hiring
   * is on this screen at the moment it is anywhere else. A terminated agent is
   * left out because it cannot be given anything; the runtime drops its place
   * on every plugin when it goes.
   */
  const members = agents.filter(
    (agent) => agent.groupId === group?.id && agent.lifecycle !== "terminated",
  );

  const save = async () => {
    setBusy(true);
    setStatus(null);
    try {
      const saved = group
        ? await api.updateGroup(group.id, draft())
        : await api.createGroup(draft());
      await refreshAgents();
      // Read the limits back rather than leaving what was typed. The runtime
      // clamps what it stores, so a relay depth of 40 is kept as 16, and
      // closing over a box still reading 40 would say something untrue about
      // what this crew runs on.
      setLimits(
        Object.fromEntries(LIMITS.map((field) => [field.key, asText(saved.limits[field.key])])),
      );
      setApiKey(null);
      onClose();
    } catch (caught) {
      setStatus({ tone: "error", text: errorMessage(caught) });
      setBusy(false);
    }
  };

  const test = async () => {
    setBusy(true);
    setStatus(null);
    try {
      // Sends what is on screen and resolves it over the app settings, so this
      // reports on what this crew's next turn would actually do.
      setStatus({ tone: "ok", text: await api.testGroupConnection(group?.id ?? null, draft()) });
    } catch (caught) {
      setStatus({ tone: "error", text: errorMessage(caught) });
    } finally {
      setBusy(false);
    }
  };

  /**
   * Start fresh: the crew stays, everything it has accumulated goes.
   *
   * Transcripts, schedules, both stores and spend together, because clearing
   * only the transcript left agents acting on a memory of a conversation that
   * no longer existed and keeping appointments nobody could see the reason for.
   * The working notes go for the same reason the memories do, and they are the
   * half that says what the crew was in the middle of: left behind, every agent
   * opens tomorrow waiting on somebody about a conversation nobody can read.
   */
  const clear = async () => {
    if (!group) return;
    setBusy(true);
    setStatus(null);
    try {
      const gone = await api.clearGroup(group.id);
      setCleared(gone);
      setConfirmClear(false);
      await refreshAgents();
    } catch (caught) {
      setStatus({ tone: "error", text: errorMessage(caught) });
    } finally {
      setBusy(false);
    }
  };

  /**
   * The group, and whoever is still standing in it.
   *
   * One button rather than two, because "delete this crew" is one intent an
   * operator arrives with and an empty group is the same act with nothing in
   * the way. Two buttons would mean the one for a populated group had a single
   * outcome, which was an error telling the operator to go and delete four
   * agents by hand first.
   *
   * What changes is the confirmation, which names the count before it is
   * pressed. The count is the roster's, so a group emptied elsewhere since the
   * dialog opened takes the plain delete and a stale zero is refused by the
   * runtime rather than quietly widened.
   */
  const remove = async () => {
    if (!group) return;
    setBusy(true);
    setStatus(null);
    try {
      if (crew > 0) await api.disbandGroup(group.id);
      else await api.deleteGroup(group.id);
      await refreshAgents();
      onClose();
    } catch (caught) {
      // The remaining failure is the first group, which cannot go because every
      // agent has to be in one. The message from Rust says so, and is shown as
      // written.
      setStatus({ tone: "error", text: errorMessage(caught) });
      setBusy(false);
    }
  };

  /** Choosing an endpoint is also choosing to pay with a key, exactly as it is
   *  in Settings: a group left following the app would have the operator fill
   *  in a URL that nothing read, with no error to explain it. */
  const choose = (preset: Preset) => {
    setProvider("compatible");
    setBaseUrl(preset.baseUrl);
    if (preset.model) setModel(preset.model);
  };

  /** What this group would run on if it said nothing, written for a person. */
  const inherited =
    settings?.provider === "chatgpt"
      ? `ChatGPT subscription · ${settings.subscriptionModel}`
      : settings?.provider === "claude"
        ? "Claude subscription · the model Claude is set to"
        : `${providerFor(settings?.baseUrl ?? "")?.name ?? settings?.baseUrl ?? "the app endpoint"} · ${settings?.defaultModel ?? ""}`;

  const onSubscription = provider === "chatgpt";
  const onClaude = provider === "claude";
  const onEndpoint = provider === "compatible";

  /**
   * The same sentence for whatever this crew is actually set to.
   *
   * Read from what is on screen rather than from what is stored, so the
   * General pane is a summary of the edit in progress and not of the edit
   * before it.
   */
  const paying = onSubscription
    ? `ChatGPT subscription · ${subscriptionModel || settings?.subscriptionModel || ""}`
    : onClaude
      ? "Claude subscription · the model Claude is set to"
      : onEndpoint
        ? `${providerFor(baseUrl || (settings?.baseUrl ?? ""))?.name ?? baseUrl} · ${model || settings?.defaultModel || ""}`
        : `The app settings · ${inherited}`;

  const setLimitCount = LIMITS.filter((field) => (limits[field.key] ?? "").trim()).length;

  return (
    <div className="scrim">
      <button type="button" className="scrim__close" aria-label="Close dialog" onClick={onClose} />
      <div
        className="dialog dialog--settings"
        role="dialog"
        aria-modal="true"
        aria-label={group ? "Group settings" : "New group"}
      >
        <div className="settings__head">
          <h2 className="dialog__title">{group ? name || group.name : "New group"}</h2>
          <p className="dialog__lede" style={{ margin: 0 }}>
            Agents in different groups cannot see or message each other. Everything below is
            inherited from the app settings unless this group sets it.
          </p>
        </div>

        <div className="settings__body">
          <div
            className="settings__nav"
            role="tablist"
            aria-orientation="vertical"
            aria-label="Group sections"
          >
            {SECTIONS.map((key) => (
              <button
                key={key}
                type="button"
                role="tab"
                className="settings__tab"
                aria-selected={section === key}
                disabled={NEEDS_GROUP.includes(key) && !group}
                onClick={() => setSection(key)}
              >
                {SECTION_LABELS[key]}
              </button>
            ))}
          </div>

          {/* The board brings its own scrolling and wants the whole pane, so
              the pane stops being a padded column of prose for that one
              section. See `.settings__pane--board`. */}
          <div
            className={
              section === "activity" ? "settings__pane settings__pane--board" : "settings__pane"
            }
          >
            {section === "general" && (
              <>
                <h3 className="settings__title">General</h3>
                <p className="settings__lede">What this crew is called, and what it runs on.</p>

                <label className="field">
                  <span className="field__label">Name</span>
                  <input
                    className="input input--mono"
                    ref={nameRef}
                    value={name}
                    maxLength={48}
                    placeholder="Research"
                    onChange={(event) => setName(event.target.value)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" && name.trim()) void save();
                    }}
                  />
                </label>

                {/* A summary rather than more controls. Whatever is set lives
                    one tab away, and the question this pane is opened with is
                    what this crew is doing now. */}
                <div className="field">
                  <span className="field__label">Turns paid for by</span>
                  <span className="field__hint">{paying}</span>
                </div>

                <div className="field">
                  <span className="field__label">Limits</span>
                  <span className="field__hint">
                    {setLimitCount === 0
                      ? "The app's, all five."
                      : `${setLimitCount} of five set here; the rest are the app's.`}
                  </span>
                </div>

                {group && (
                  <div className="field">
                    <span className="field__label">Agents</span>
                    <span className="field__hint">
                      {group.agentCount === 0
                        ? "Nobody yet. Agents are hired into a group, and cannot see out of it."
                        : `${group.agentCount} agent${group.agentCount === 1 ? "" : "s"}, none of whom can see or message anybody outside this group.`}
                    </span>
                  </div>
                )}
              </>
            )}

            {section === "provider" && (
              <>
                <h3 className="settings__title">Provider</h3>
                <p className="settings__lede">
                  Who pays for this crew's turns, and which model answers them. One crew can run on
                  a local server while another spends the subscription.
                </p>

                <button
                  type="button"
                  className="preset"
                  aria-current={provider === INHERIT}
                  onClick={() => setProvider(INHERIT)}
                >
                  <span className="preset__text">
                    <span className="preset__name">Follow the app settings</span>
                    <span className="preset__url">{inherited}</span>
                  </span>
                </button>

                {/* Its own block, above the endpoint list, because it is not an
                    endpoint. The sign-in itself belongs to the app: it is one
                    credential on this machine, and a group only decides whether
                    to spend it. */}
                <div className="preset preset--plain" aria-current={onSubscription}>
                  <span className="preset__text">
                    <span className="preset__name">ChatGPT subscription</span>
                    <span className="preset__url">
                      {subscription?.signedIn
                        ? `${subscription.email || "signed in"}${
                            subscription.plan ? ` · ${planLabel(subscription.plan)}` : ""
                          }`
                        : "Not signed in."}
                    </span>
                  </span>
                  {subscription?.signedIn &&
                    (onSubscription ? (
                      <span className="preset__state" data-ready="true">
                        In use
                      </span>
                    ) : (
                      <button
                        type="button"
                        className="btn btn--small"
                        aria-label="Use the ChatGPT subscription"
                        disabled={busy}
                        onClick={() => setProvider("chatgpt")}
                      >
                        Use it
                      </button>
                    ))}
                </div>

                {/* Said in both states, not only when nobody is signed in.
                    Whichever it is, this row is the whole subscription as far
                    as a group is concerned, and an operator whose turns are
                    being refused reads "signed in" here and has nothing to
                    press: signing out and back in is two panes away and this
                    was the only place saying so. */}
                <p className="hint">
                  Signing in and out happens in Settings → Provider. There is one sign-in on this
                  machine and every group spends the same one.
                </p>

                {onSubscription && (
                  <SubscriptionModel
                    value={subscriptionModel}
                    models={settings?.subscriptionModels ?? []}
                    onChange={setSubscriptionModel}
                    inherit={`Inherit · ${settings?.subscriptionModel ?? ""}`}
                    hint="Used by every agent in this group that does not name its own model. A subscription has an hourly quota rather than a per-token bill, and every crew spending it shares that quota."
                  />
                )}

                {/* Beside the other subscription and above the endpoint list,
                    for the reason that one gives. No sign-in state to draw and
                    no model to pick: both belong to the program, and a group
                    only decides whether its turns are spent through it. */}
                <div className="preset preset--plain" aria-current={onClaude}>
                  <span className="preset__text">
                    <span className="preset__name">Claude subscription</span>
                    <span className="preset__url">
                      {capabilities.claudeProvider
                        ? "Runs the claude program, on whatever it is signed in to"
                        : "Runs the claude program where you signed in, which is your own machine, so a server cannot offer it"}
                    </span>
                  </span>
                  {!capabilities.claudeProvider ? (
                    <span className="preset__state" data-ready="false">
                      Not on a server
                    </span>
                  ) : onClaude ? (
                    <span className="preset__state" data-ready="true">
                      In use
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="btn btn--small"
                      aria-label="Use the Claude subscription"
                      disabled={busy}
                      onClick={() => setProvider("claude")}
                    >
                      Use it
                    </button>
                  )}
                </div>

                {/* Said here rather than only in Settings, for the reason the
                    subscription's hint above is said in both states: this row
                    is the whole of Claude as far as a group is concerned, and
                    an operator whose crew is ignoring its model field has
                    nothing else on screen that would explain it. */}
                <p className="hint">
                  Which model runs, and which account pays, are Claude's own settings. A model named
                  on this group or on one of its agents is not used while Claude is the provider.
                </p>

                <p className="settings__lede" style={{ marginTop: "1.4rem" }}>
                  Or any OpenAI-compatible endpoint. The ones below are spelled correctly; choosing
                  one fills in the fields under it, and anything else can be typed in.
                </p>

                <ProviderPresets
                  baseUrl={baseUrl || (settings?.baseUrl ?? "")}
                  active={onEndpoint}
                  keySet={Boolean(group?.apiKeySet || settings?.apiKeySet)}
                  loopback={capabilities.loopbackEndpoints}
                  onChoose={choose}
                />

                <label className="field" style={{ marginTop: "1.1rem" }}>
                  <span className="field__label">Inference endpoint</span>
                  <input
                    className="input input--mono"
                    value={baseUrl}
                    placeholder={settings?.baseUrl || "inherit"}
                    onChange={(event) => setBaseUrl(event.target.value)}
                  />
                  <span className="field__hint">
                    Used when a key is paying. Blank follows the app.
                  </span>
                </label>

                <label className="field">
                  <span className="field__label">API key</span>
                  <input
                    className="input input--mono"
                    type="password"
                    value={apiKey ?? ""}
                    placeholder={group?.apiKeySet ? `set · ${group.apiKeyHint}` : "inherit"}
                    autoComplete="off"
                    onChange={(event) => setApiKey(event.target.value)}
                  />
                  <span className="field__hint">
                    Only needed when this group's endpoint uses a different key. Leave blank to keep
                    the stored one; clear it to go back to the app's.
                  </span>
                </label>

                <label className="field">
                  <span className="field__label">Default model</span>
                  <input
                    className="input input--mono"
                    value={model}
                    placeholder={settings?.defaultModel || "inherit"}
                    onChange={(event) => setModel(event.target.value)}
                  />
                  <span className="field__hint">
                    Used by every agent in this group that does not name its own, when a key is
                    paying.
                  </span>
                </label>

                <label className="field">
                  <span className="field__label">Give up on a call after</span>
                  <input
                    className="input input--mono"
                    inputMode="numeric"
                    value={timeout}
                    placeholder={`${settings?.requestTimeoutSecs ?? 120} seconds`}
                    onChange={(event) => setTimeoutSecs(event.target.value.replace(/[^0-9]/g, ""))}
                  />
                  <span className="field__hint">
                    Seconds to wait for a model call. A crew on a slow local model wants more than
                    one on a hosted endpoint.
                  </span>
                </label>

                <div className="settings__probe">
                  <button type="button" className="btn" disabled={busy} onClick={() => void test()}>
                    Test connection
                  </button>
                  <span className="hint">Tests what is on screen. Save to keep it.</span>
                </div>
              </>
            )}

            {section === "limits" && (
              <>
                <h3 className="settings__title">Limits</h3>
                <p className="settings__lede">
                  Agents that message each other do not stop on their own. These bounds decide when
                  a conversation started in this group ends, and every agent is told why when it
                  hits one. Blank follows the app.
                </p>

                {LIMITS.map((field) => (
                  <label className="field" key={field.key}>
                    <span className="field__label">{field.label}</span>
                    <input
                      className="input input--mono input--number"
                      type="number"
                      min={field.min}
                      max={field.max}
                      value={limits[field.key] ?? ""}
                      placeholder={String(settings?.limits[field.key] ?? "")}
                      onChange={(event) =>
                        setLimits((current) => ({
                          ...current,
                          [field.key]: event.target.value.replace(/[^0-9]/g, ""),
                        }))
                      }
                    />
                    <span className="field__hint">{field.hint}</span>
                  </label>
                ))}
              </>
            )}

            {/* What a crew can reach is signed in to once, here, and handing it
                out is a second decision: a plugin can be narrowed to named
                agents, because the one that files issues has no business
                holding the account that issues refunds. Credentials are still
                the whole group's. */}
            {section === "plugins" && group && (
              <>
                <h3 className="settings__title">Plugins</h3>
                <p className="settings__lede">
                  Sign in once, on behalf of this group, then choose which agents get it. Every
                  agent is the default; narrow the ones that reach money or production. None of them
                  ever holds the sign-in.
                </p>
                <PluginList groupId={group.id} crew={members} />
                <CredentialList groupId={group.id} />
              </>
            )}

            {/* Its own section rather than a third panel under Plugins. A
                plugin is a server this crew signs in to and a repository is a
                directory on this machine that it writes in: they share a shape
                (given to the crew, then handed to named agents) and nothing
                else, and stacked in one pane the operator scrolled past two
                sign-in panels to reach the one about their own source. */}
            {section === "repositories" && group && (
              <>
                <h3 className="settings__title">Repositories</h3>
                <p className="settings__lede">
                  Add a codebase for this crew, then assign it to an agent in that agent’s profile.
                  Repositories and coding tools run on the connected backend.
                </p>
                <RepositoryList groupId={group.id} crew={members} />
              </>
            )}

            {section === "activity" && group && (
              <>
                <h3 className="settings__title">Activity</h3>
                <p className="settings__lede">
                  Who spoke to whom in this crew, in order, one board per run. Click any arrow to
                  read the message. Read when this pane was opened; reopen it for anything since.
                </p>
                <GroupActivity group={group.id} />
              </>
            )}

            {cleared && (
              <div className="banner" style={{ margin: "0.9rem 0 0" }}>
                <span>
                  Reset: {cleared.messages} message{cleared.messages === 1 ? "" : "s"},{" "}
                  {cleared.routines} routine{cleared.routines === 1 ? "" : "s"}, {cleared.notes}{" "}
                  memor{cleared.notes === 1 ? "y" : "ies"}, {cleared.workingNotes} working note
                  {cleared.workingNotes === 1 ? "" : "s"}, and {cleared.calls} recorded call
                  {cleared.calls === 1 ? "" : "s"}.
                </span>
              </div>
            )}
          </div>
        </div>

        {/* What the second click costs, written where the operator is already
            looking. The button below carries the count; this is the part a
            count does not say: the machines are rented, and destroying them is
            the half of a disband that cannot be undone. */}
        {group && confirmDelete && crew > 0 && (
          <div className="banner banner--error" style={{ margin: "0.75rem 1.35rem 0" }}>
            <span>
              {crew} agent{crew === 1 ? "" : "s"} go with the group: their computers, browsers,
              memories and schedules are destroyed. What they said stays readable.
            </span>
          </div>
        )}

        {status && (
          <div
            className={status.tone === "error" ? "banner banner--error" : "banner banner--ok"}
            style={{ margin: "0.75rem 1.35rem 0" }}
          >
            <span>{status.text}</span>
          </div>
        )}

        <div className="settings__foot">
          {group &&
            (confirmDelete ? (
              <>
                <button
                  type="button"
                  className="btn btn--danger"
                  disabled={busy}
                  onClick={() => void remove()}
                >
                  {crew > 0
                    ? `Delete ${group.name} and ${crew} agent${crew === 1 ? "" : "s"}`
                    : `Delete ${group.name}`}
                </button>
                <button
                  type="button"
                  className="btn btn--ghost"
                  onClick={() => setConfirmDelete(false)}
                >
                  Keep
                </button>
              </>
            ) : (
              <button
                type="button"
                className="btn btn--danger"
                onClick={() => setConfirmDelete(true)}
                title={
                  crew > 0
                    ? "Deletes the group and every agent in it, along with their computers and browsers. What they said stays readable."
                    : "Deletes the group. It holds no agents."
                }
              >
                Delete
              </button>
            ))}
          {group &&
            !confirmDelete &&
            (confirmClear ? (
              <>
                <button
                  type="button"
                  className="btn btn--danger"
                  disabled={busy}
                  onClick={() => void clear()}
                >
                  Reset every agent
                </button>
                <button
                  type="button"
                  className="btn btn--ghost"
                  onClick={() => setConfirmClear(false)}
                >
                  Keep them
                </button>
              </>
            ) : (
              <button
                type="button"
                className="btn btn--ghost"
                disabled={busy}
                onClick={() => setConfirmClear(true)}
                title="Resets every agent in this group: transcripts, routines, memories, working notes and the spend counter. The agents and their computers stay."
              >
                {cleared === null ? "Start fresh" : "Reset"}
              </button>
            ))}
          <span style={{ flex: 1 }} />
          {/* Cancel only while there is something to cancel. With nothing
              staged this button closes a dialog, and saying otherwise invites
              the operator to think the plugin they just signed in goes with
              it. */}
          <button type="button" className="btn" onClick={onClose}>
            {dirty ? "Cancel" : "Close"}
          </button>
          {dirty && (
            <button
              type="button"
              className="btn btn--primary"
              disabled={busy || !name.trim()}
              onClick={() => void save()}
            >
              {group ? "Save" : "Create"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
