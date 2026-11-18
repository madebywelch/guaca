import { useEffect, useRef, useState } from "react";

import { useDismiss } from "../lib/dismiss";
import { api } from "../lib/ipc";
import { LIMITS } from "../lib/limits";
import { type Provider as Preset, planLabel, providerFor } from "../lib/providers";
import { clockTime } from "../lib/routine";
import { useStore } from "../lib/store";
import {
  errorMessage,
  type Group,
  type GroupDraft,
  type GroupReset,
  type GroupTidy,
  type Provider,
  type ReasoningEffort,
  type SubscriptionStatus,
} from "../lib/types";
import { CredentialList } from "./CredentialList";
import { GroupActivity } from "./GroupActivity";
import { GroupTransfer } from "./GroupTransfer";
import { Permissions } from "./Permissions";
import { PluginList } from "./PluginList";
import { ProviderPresets, SubscriptionModel } from "./ProviderFields";
import { SkillList } from "./SkillList";
import { ToolList } from "./ToolList";

const HOURS = Array.from({ length: 24 }, (_, hour) => hour);

interface Props {
  group?: Group;
  onClose: () => void;
  onSection?: (section: string) => void;
}

const SECTIONS = [
  "general",
  "provider",
  "limits",
  "permissions",
  "plugins",
  "tools",
  "secrets",
  "skills",
  "activity",
  "transfer",
] as const;

type Section = (typeof SECTIONS)[number];

const SECTION_LABELS: Record<Section, string> = {
  general: "General",
  provider: "Provider",
  limits: "Limits",
  permissions: "Permissions",
  plugins: "Connectors",
  tools: "Tools",
  secrets: "Secrets",
  skills: "Skills",
  activity: "Activity",
  transfer: "Import / export",
};

const NEEDS_GROUP: readonly Section[] = ["plugins", "secrets", "skills", "activity", "permissions"];

const INHERIT = "inherit";

const asText = (value: number | null | undefined) => (value === null ? "" : String(value ?? ""));
const asNumber = (text: string) => (text.trim() ? Number(text) : null);

export function GroupEditor({ group, onClose, onSection }: Props) {
  const refreshAgents = useStore((s) => s.refreshAgents);
  const settings = useStore((s) => s.settings);
  const capabilities = useStore((s) => s.capabilities);
  const agents = useStore((s) => s.agents);

  const [section, setSection] = useState<Section>("general");
  const [visitedPermissions, setVisitedPermissions] = useState(false);
  useEffect(() => {
    if (section === "permissions") setVisitedPermissions(true);
  }, [section]);
  useEffect(() => onSection?.(section), [section, onSection]);
  const [name, setName] = useState(group?.name ?? "");
  const [provider, setProvider] = useState<Provider | typeof INHERIT>(
    group?.inference.provider ?? INHERIT,
  );
  const [baseUrl, setBaseUrl] = useState(group?.inference.baseUrl ?? "");
  const [model, setModel] = useState(group?.inference.defaultModel ?? "");
  const [subscriptionModel, setSubscriptionModel] = useState(
    group?.inference.subscriptionModel ?? "",
  );
  const [reasoningEffort, setReasoningEffort] = useState<ReasoningEffort | null>(
    group?.inference.reasoningEffort ?? null,
  );
  const [timeout, setTimeoutSecs] = useState(asText(group?.inference.requestTimeoutSecs));
  const [apiKey, setApiKey] = useState<string | null>(null);
  const [tidy, setTidy] = useState<GroupTidy>(group?.tidy ?? { on: false, hour: 3 });
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

  useDismiss(onClose);

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
      reasoningEffort,
      requestTimeoutSecs: asNumber(timeout),
    },
    ...(apiKey !== null ? { apiKey } : {}),
    limits: {
      maxHops: asNumber(limits.maxHops ?? ""),
      maxStepsPerRun: asNumber(limits.maxStepsPerRun ?? ""),
      maxFanoutPerCall: asNumber(limits.maxFanoutPerCall ?? ""),
      maxSendsPerPair: asNumber(limits.maxSendsPerPair ?? ""),
      maxToolRounds: asNumber(limits.maxToolRounds ?? ""),
    },
    tidy,
  });

  const dirty =
    !group ||
    name !== group.name ||
    provider !== (group.inference.provider ?? INHERIT) ||
    baseUrl !== (group.inference.baseUrl ?? "") ||
    model !== (group.inference.defaultModel ?? "") ||
    subscriptionModel !== (group.inference.subscriptionModel ?? "") ||
    reasoningEffort !== (group.inference.reasoningEffort ?? null) ||
    timeout !== asText(group.inference.requestTimeoutSecs) ||
    apiKey !== null ||
    LIMITS.some((field) => limits[field.key] !== asText(group.limits[field.key])) ||
    tidy.on !== group.tidy.on ||
    tidy.hour !== group.tidy.hour;

  const crew = group?.agentCount ?? 0;

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
      setStatus({ tone: "ok", text: await api.testGroupConnection(group?.id ?? null, draft()) });
    } catch (caught) {
      setStatus({ tone: "error", text: errorMessage(caught) });
    } finally {
      setBusy(false);
    }
  };

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
      setStatus({ tone: "error", text: errorMessage(caught) });
      setBusy(false);
    }
  };

  const choose = (preset: Preset) => {
    setProvider("compatible");
    setBaseUrl(preset.baseUrl);
    if (preset.model) setModel(preset.model);
  };

  const inherited =
    settings?.provider === "chatgpt"
      ? `ChatGPT subscription · ${settings.subscriptionModel}`
      : settings?.provider === "claude"
        ? "Claude subscription · the model Claude is set to"
        : `${providerFor(settings?.baseUrl ?? "")?.name ?? settings?.baseUrl ?? "the app endpoint"} · ${settings?.defaultModel ?? ""}`;

  const onSubscription = provider === "chatgpt";
  const onClaude = provider === "claude";
  const onEndpoint = provider === "compatible";

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

          <div
            className={
              section === "activity" ? "settings__pane settings__pane--board" : "settings__pane"
            }
          >
            {section === "transfer" && <GroupTransfer group={group} />}
            {section === "general" && (
              <>
                <h3 className="settings__title">General</h3>
                <p className="settings__lede">What this group is called, and what it runs on.</p>

                <label className="field">
                  <span className="field__label">Name</span>
                  <input
                    className="input"
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

                <div className="field field--row">
                  <label className="field--row">
                    <input
                      type="checkbox"
                      checked={tidy.on}
                      onChange={(event) => setTidy({ ...tidy, on: event.target.checked })}
                    />
                    <span className="field__label">Tidy memory overnight</span>
                  </label>
                  <span className="field__label">at</span>
                  <select
                    className="input"
                    aria-label="Hour"
                    value={tidy.hour}
                    onChange={(event) => setTidy({ ...tidy, hour: Number(event.target.value) })}
                  >
                    {HOURS.map((hour) => (
                      <option key={hour} value={hour}>
                        {clockTime(new Date(2000, 0, 1, hour).getTime())}
                      </option>
                    ))}
                  </select>
                </div>

                {group && (
                  <div className="field">
                    <span className="field__label">Agents</span>
                    <span className="field__hint">
                      {group.agentCount === 0
                        ? "Nobody yet."
                        : `${group.agentCount} agent${group.agentCount === 1 ? "" : "s"}.`}
                    </span>
                  </div>
                )}
              </>
            )}

            {section === "provider" && (
              <>
                <h3 className="settings__title">Provider</h3>
                <p className="settings__lede">
                  Who pays for this group's turns, and which model answers them. One group can run
                  on a local server while another spends the subscription.
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
                        className="btn btn--compact"
                        aria-label="Use the ChatGPT subscription"
                        disabled={busy}
                        onClick={() => setProvider("chatgpt")}
                      >
                        Use it
                      </button>
                    ))}
                </div>

                <p className="hint">
                  Signing in and out happens in Settings → Provider. There is one sign-in on this
                  machine and every group spends the same one.
                </p>

                {(onSubscription || (provider === INHERIT && settings?.provider === "chatgpt")) && (
                  <SubscriptionModel
                    value={subscriptionModel}
                    effort={reasoningEffort}
                    onEffortChange={setReasoningEffort}
                    inheritedModel={settings?.subscriptionModel}
                    inheritedEffort={settings?.reasoningEffort ?? "auto"}
                    effortInherit="Inherit"
                    models={settings?.subscriptionModels ?? []}
                    onChange={setSubscriptionModel}
                    inherit={`Inherit · ${settings?.subscriptionModel ?? ""}`}
                    title="The default for this group. An agent chooses its own in its profile. Every agent on this subscription shares its quota."
                  />
                )}

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
                      className="btn btn--compact"
                      aria-label="Use the Claude subscription"
                      disabled={busy}
                      onClick={() => setProvider("claude")}
                    >
                      Use it
                    </button>
                  )}
                </div>

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
                    className="input"
                    value={baseUrl}
                    placeholder={settings?.baseUrl || "inherit"}
                    title="Used when a key is paying. Blank follows the app."
                    onChange={(event) => setBaseUrl(event.target.value)}
                  />
                </label>

                <label className="field">
                  <span className="field__label">API key</span>
                  <input
                    className="input"
                    type="password"
                    value={apiKey ?? ""}
                    placeholder={group?.apiKeySet ? `set · ${group.apiKeyHint}` : "inherit"}
                    autoComplete="off"
                    title="Only when this group's endpoint takes a different key. Blank keeps the stored one; clear it to go back to the app's."
                    onChange={(event) => setApiKey(event.target.value)}
                  />
                </label>

                <label className="field">
                  <span className="field__label">Default model</span>
                  <input
                    className="input"
                    value={model}
                    placeholder={settings?.defaultModel || "inherit"}
                    title="Every agent in this group that does not name its own, when a key is paying."
                    onChange={(event) => setModel(event.target.value)}
                  />
                </label>

                <label className="field">
                  <span className="field__label">Give up on a call after</span>
                  <input
                    className="input"
                    inputMode="numeric"
                    value={timeout}
                    placeholder={`${settings?.requestTimeoutSecs ?? 120} seconds`}
                    title="Seconds to wait for a model call. A group on a slow local model wants more than one on a hosted endpoint."
                    onChange={(event) => setTimeoutSecs(event.target.value.replace(/[^0-9]/g, ""))}
                  />
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
                    <span className="field__label" title={field.hint}>
                      {field.label}
                    </span>
                    <input
                      className="input input--number"
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
                  </label>
                ))}
              </>
            )}

            {section === "plugins" && group && (
              <>
                <h3 className="settings__title">Connectors</h3>
                <p className="settings__lede">
                  MCP servers this group can reach. Sign in once, on behalf of the group, then
                  choose which agents get each one and which of its tools they may call. Every agent
                  is the default; narrow the ones that reach money or production. None of them ever
                  holds the sign-in.
                </p>
                <PluginList groupId={group.id} crew={members} />
              </>
            )}

            {section === "tools" && (
              <>
                <h3 className="settings__title">Tools</h3>
                <p className="settings__lede">
                  The functions every agent can call, Guaca's own, described in the words the agent
                  is given. Each connector adds its own tools, listed under Connectors. Skills are
                  not tools: they are instructions an agent reads with one.
                </p>
                <ToolList />
              </>
            )}

            {section === "secrets" && group && (
              <>
                <h3 className="settings__title">Secrets</h3>
                <p className="settings__lede">
                  Save a service token and choose the agents that can use it. Selected agents get
                  the variable in their computer commands.
                </p>
                <CredentialList groupId={group.id} crew={members} />
              </>
            )}

            {group && visitedPermissions && (
              <div hidden={section !== "permissions"}>
                <Permissions groupId={group.id} />
              </div>
            )}

            {section === "skills" && group && (
              <>
                <h3 className="settings__title">Skills</h3>
                <p className="settings__lede">
                  Instructions for kinds of work this group does. Its agents can write these as they
                  work something out, and no other group can read them.
                </p>
                <SkillList scope={{ kind: "crew", groupId: group.id }} />
              </>
            )}

            {section === "activity" && group && (
              <>
                <h3 className="settings__title">Activity</h3>
                <p className="settings__lede">
                  Who spoke to whom in this group, in order, one board per run. Click any arrow to
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
                  className="btn btn--quiet"
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
                  className="btn btn--quiet"
                  onClick={() => setConfirmClear(false)}
                >
                  Keep them
                </button>
              </>
            ) : (
              <button
                type="button"
                className="btn btn--quiet"
                disabled={busy}
                onClick={() => setConfirmClear(true)}
                title="Resets every agent in this group: transcripts, routines, memories, working notes and the spend counter. The agents and their computers stay."
              >
                {cleared === null ? "Start fresh" : "Reset"}
              </button>
            ))}
          <span style={{ flex: 1 }} />
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
