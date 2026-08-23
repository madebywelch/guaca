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
import { PluginList } from "./PluginList";
import { ProviderPresets, SubscriptionModel } from "./ProviderFields";

interface Props {
  /** Absent means create. */
  group?: Group;
  onClose: () => void;
}

const SECTIONS = ["general", "provider", "limits", "plugins"] as const;

type Section = (typeof SECTIONS)[number];

const SECTION_LABELS: Record<Section, string> = {
  general: "General",
  provider: "Provider",
  limits: "Limits",
  plugins: "Plugins",
};

/** What a group says when it has no opinion about who pays. */
const INHERIT = "inherit";

/** A number an operator may leave blank, as it is typed and as it is stored. */
const asText = (value: number | null | undefined) => (value === null ? "" : String(value ?? ""));
const asNumber = (text: string) => (text.trim() ? Number(text) : null);

export function GroupEditor({ group, onClose }: Props) {
  const refreshAgents = useStore((s) => s.refreshAgents);
  const settings = useStore((s) => s.settings);

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

  /** Live agents. Terminated ones are not in the count and are not deleted twice. */
  const crew = group?.agentCount ?? 0;

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
   * Transcripts, schedules, memories and spend together, because clearing only
   * the transcript left agents acting on a memory of a conversation that no
   * longer existed and keeping appointments nobody could see the reason for.
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
      : `${providerFor(settings?.baseUrl ?? "")?.name ?? settings?.baseUrl ?? "the app endpoint"} · ${settings?.defaultModel ?? ""}`;

  const onSubscription = provider === "chatgpt";
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
                // A group that does not exist yet has nothing to connect a
                // plugin to, and nowhere to put a credential.
                disabled={key === "plugins" && !group}
                onClick={() => setSection(key)}
              >
                {SECTION_LABELS[key]}
              </button>
            ))}
          </div>

          <div className="settings__pane">
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
                        : "Not signed in. Settings → Provider is where that happens."}
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
                        disabled={busy}
                        onClick={() => setProvider("chatgpt")}
                      >
                        Use it
                      </button>
                    ))}
                </div>

                {onSubscription && (
                  <SubscriptionModel
                    value={subscriptionModel}
                    models={settings?.subscriptionModels ?? []}
                    onChange={setSubscriptionModel}
                    inherit={`Inherit · ${settings?.subscriptionModel ?? ""}`}
                    hint="Used by every agent in this group that does not name its own model. A subscription has an hourly quota rather than a per-token bill, and every crew spending it shares that quota."
                  />
                )}

                <p className="settings__lede" style={{ marginTop: "1.4rem" }}>
                  Or any OpenAI-compatible endpoint. The ones below are spelled correctly; choosing
                  one fills in the fields under it, and anything else can be typed in.
                </p>

                <ProviderPresets
                  baseUrl={baseUrl || (settings?.baseUrl ?? "")}
                  active={onEndpoint}
                  keySet={Boolean(group?.apiKeySet || settings?.apiKeySet)}
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

            {/* What a crew can reach belongs to the crew, not to any one agent
                in it: a plugin's sign-in and a machine's credentials are both
                handed to everybody here. */}
            {section === "plugins" && group && (
              <>
                <h3 className="settings__title">Plugins</h3>
                <p className="settings__lede">
                  Sign in once, on behalf of this group. Every agent in it can then call that
                  service's tools, and none of them ever holds the sign-in.
                </p>
                <PluginList groupId={group.id} />
                <CredentialList groupId={group.id} />
              </>
            )}

            {cleared && (
              <div className="banner" style={{ margin: "0.9rem 0 0" }}>
                <span>
                  Reset: {cleared.messages} message{cleared.messages === 1 ? "" : "s"},{" "}
                  {cleared.routines} routine{cleared.routines === 1 ? "" : "s"}, {cleared.notes}{" "}
                  memor{cleared.notes === 1 ? "y" : "ies"}, and {cleared.calls} recorded call
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
                title="Resets every agent in this group: transcripts, routines and memories, and the spend counter. The agents and their computers stay."
              >
                {cleared === null ? "Start fresh" : "Reset"}
              </button>
            ))}
          <span style={{ flex: 1 }} />
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="btn btn--primary"
            disabled={busy || !name.trim()}
            onClick={() => void save()}
          >
            {group ? "Save" : "Create"}
          </button>
        </div>
      </div>
    </div>
  );
}
