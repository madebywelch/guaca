/**
 * Settings, as eight places rather than one scroll.
 *
 * Every piece of state lives here, in the shell, and the panes are given values
 * and setters. That is not tidiness: the shell is unmounted when the dialog
 * closes, so state held by a pane would be discarded every time the operator
 * changed section, and typing an endpoint, glancing at Limits and coming back
 * would silently lose the endpoint.
 *
 * Save and Test stay in the foot, outside the scrolling half, for the same
 * reason they used to be at the bottom of one column: they act on everything,
 * not on the section that happens to be open. Test deliberately sends what is
 * on screen rather than what is stored, and Save deliberately does not close.
 */

import { useEffect, useRef, useState } from "react";

import { applyAppearance, resolveSurface } from "../lib/appearance";
import { api } from "../lib/ipc";
import { BINDINGS, comboLabel, SURFACES } from "../lib/keybinds";
import { NOTIFY_KINDS, type NotifyKind, type SurfaceMode, UI_SCALES } from "../lib/prefs";
import { PROVIDERS, providerFor, providerReady } from "../lib/providers";
import { useStore } from "../lib/store";
import { errorMessage, type GuardLimits } from "../lib/types";

interface Props {
  onClose: () => void;
  /** Which pane to open on. The palette and the missing-key banner both point
   *  at a specific one, and landing on General to hunt for it is a step. */
  section?: Section;
}

const SECTIONS = [
  "general",
  "provider",
  "limits",
  "machines",
  "appearance",
  "notifications",
  "shortcuts",
  "about",
] as const;

export type Section = (typeof SECTIONS)[number];

const SECTION_LABELS: Record<Section, string> = {
  general: "General",
  provider: "Provider",
  limits: "Limits",
  machines: "Machines",
  appearance: "Appearance",
  notifications: "Notifications",
  shortcuts: "Shortcuts",
  about: "About",
};

interface LimitField {
  key: keyof GuardLimits;
  label: string;
  hint: string;
  min: number;
  max: number;
}

const LIMITS: LimitField[] = [
  {
    key: "maxStepsPerRun",
    label: "Model calls per conversation",
    hint: "The hard ceiling on spend. One conversation is your message plus everything it sets off.",
    min: 1,
    max: 500,
  },
  {
    key: "maxToolRounds",
    label: "Tool calls per turn",
    hint: "How many times an agent can act and look again within one turn. Working a browser is a loop of read, click, read again, so this needs room.",
    min: 1,
    max: 100,
  },
  {
    key: "maxHops",
    label: "Relay depth",
    hint: "How far a message can travel from you. A relays to B relays to C is two hops.",
    min: 1,
    max: 16,
  },
  {
    key: "maxSendsPerPair",
    label: "Messages between any two agents",
    hint: "Stops two agents from talking to each other indefinitely.",
    min: 1,
    max: 50,
  },
  {
    key: "maxFanoutPerCall",
    label: "Recipients per send",
    hint: "How many agents one message can go to at once.",
    min: 1,
    max: 64,
  },
];

const SURFACE_LABELS: Record<SurfaceMode, string> = {
  light: "Paper",
  dark: "Ink",
  system: "Follow the system",
};

const NOTIFY_COPY: Record<NotifyKind, { label: string; hint: string }> = {
  approval: {
    label: "An agent needs permission",
    hint: "The only one that blocks: the turn that asked is parked until you answer, and gives up after ten minutes. Reaches you even while Guaca is open, if the request is in a channel you are not looking at.",
  },
  routine: {
    label: "A routine fired",
    hint: "Work that starts on a schedule rather than because you asked. It lands in whichever channel it was pointed at, which is usually not the one on screen.",
  },
  settled: {
    label: "A conversation finished",
    hint: "Every agent it reached has gone quiet. Only for the channel you were last looking at, so a busy runtime does not announce work you never opened.",
  },
  failed: {
    label: "A turn could not finish",
    hint: "The model call failed after its retries. Same channel rule as above.",
  },
};

export function SettingsDialog({ onClose, section: opening }: Props) {
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const prefs = useStore((s) => s.prefs);
  const setPrefs = useStore((s) => s.setPrefs);

  const [section, setSection] = useState<Section>(opening ?? "general");
  const [operatorName, setOperatorName] = useState(settings?.operatorName ?? "");
  const [baseUrl, setBaseUrl] = useState(settings?.baseUrl ?? "");
  const [model, setModel] = useState(settings?.defaultModel ?? "");
  const [apiKey, setApiKey] = useState("");
  const [e2bKey, setE2bKey] = useState("");
  const [idleMinutes, setIdleMinutes] = useState("");
  const [kernelKey, setKernelKey] = useState("");
  const [browserIdleMinutes, setBrowserIdleMinutes] = useState("");
  const [stealth, setStealth] = useState(settings?.browserStealth ?? false);
  const [timeout, setTimeoutSecs] = useState("");
  const [limits, setLimits] = useState<GuardLimits>(
    settings?.limits ?? {
      maxHops: 8,
      maxStepsPerRun: 60,
      maxFanoutPerCall: 8,
      maxSendsPerPair: 6,
      maxToolRounds: 24,
    },
  );
  const [status, setStatus] = useState<{ tone: "ok" | "error" | "info"; text: string } | null>(
    null,
  );
  const [busy, setBusy] = useState(false);
  const [version, setVersion] = useState<string | null>(null);
  const panelRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Focus moves into the dialog on open, as it does in every other dialog here.
  // Onto the panel rather than the first field: the first thing on this surface
  // is a choice of section, not something to type into.
  useEffect(() => {
    panelRef.current?.focus();
  }, []);

  // Read on mount rather than at startup, so nothing pays for it until the one
  // pane that shows it is opened.
  useEffect(() => {
    let live = true;
    void import("@tauri-apps/api/app")
      .then(({ getVersion }) => getVersion())
      .then((value) => {
        if (live) setVersion(value);
      })
      .catch(() => {
        // A version nobody can read is worth a dash, not a banner.
      });
    return () => {
      live = false;
    };
  }, []);

  const patch = () => ({
    baseUrl,
    defaultModel: model,
    // Omitted when blank, so saving without retyping keeps the stored key.
    ...(apiKey.trim() ? { apiKey: apiKey.trim() } : {}),
    ...(e2bKey.trim() ? { e2bApiKey: e2bKey.trim() } : {}),
    ...(idleMinutes.trim() ? { computerIdleMinutes: Number(idleMinutes) } : {}),
    ...(timeout.trim() ? { requestTimeoutSecs: Number(timeout) } : {}),
  });

  const save = async () => {
    setBusy(true);
    setStatus(null);
    try {
      const next = await api.updateSettings({ operatorName, limits, ...patch() });
      setSettings(next);
      // Read the limits back rather than leaving what was typed. The `max` on a
      // number input is advisory — a pasted or typed value sails past it — and
      // the runtime sanitises what it stores, so a relay depth of 40 is kept as
      // 16. Saying "Saved." over a box still reading 40 tells the operator
      // something untrue about what is running.
      setLimits(next.limits);
      setApiKey("");
      setKernelKey("");
      setStatus({ tone: "ok", text: "Saved." });
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setBusy(false);
    }
  };

  const test = async () => {
    setBusy(true);
    setStatus({ tone: "info", text: "Testing…" });
    try {
      // Sends what is on screen, so testing before saving does the obvious
      // thing rather than reporting on a key the operator has already replaced.
      // Deliberately without operatorName and limits: neither is something an
      // endpoint can be tested against, and a blank model would fail
      // validation here instead of reporting on the endpoint.
      const result = await api.testConnection(patch());
      setStatus({ tone: "ok", text: result });
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setBusy(false);
    }
  };

  const choose = (provider: (typeof PROVIDERS)[number]) => {
    setBaseUrl(provider.baseUrl);
    if (provider.model) setModel(provider.model);
  };

  const current = providerFor(baseUrl);

  return (
    <div className="scrim">
      {/* A real button, so dismissing by clicking away is reachable from the
          keyboard and announced, rather than being an invisible div handler. */}
      <button type="button" className="scrim__close" aria-label="Close dialog" onClick={onClose} />
      <div
        className="dialog dialog--settings"
        role="dialog"
        aria-modal="true"
        aria-label="Settings"
        tabIndex={-1}
        ref={panelRef}
      >
        <div className="settings__head">
          <h2 className="dialog__title">Settings</h2>
          <p className="dialog__lede" style={{ margin: 0 }}>
            Guaca runs entirely on this machine. The only thing it sends anywhere is what you and
            your agents type, to the endpoint you choose.
          </p>
        </div>

        <div className="settings__body">
          <div
            className="settings__nav"
            role="tablist"
            aria-orientation="vertical"
            aria-label="Settings sections"
          >
            {SECTIONS.map((key) => (
              <button
                key={key}
                type="button"
                role="tab"
                className="settings__tab"
                aria-selected={section === key}
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
                <p className="settings__lede">Who the agents think they are talking to.</p>

                <label className="field">
                  <span className="field__label">Your name</span>
                  <input
                    className="input"
                    value={operatorName}
                    placeholder="Unnamed"
                    onChange={(event) => setOperatorName(event.target.value)}
                  />
                  <span className="field__hint">
                    What every agent calls you. They are told this on every turn, so you never have
                    to introduce yourself or ask one to remember it. Leave blank and they say "the
                    operator".
                  </span>
                </label>
              </>
            )}

            {section === "provider" && (
              <>
                <h3 className="settings__title">Provider</h3>
                <p className="settings__lede">
                  Any OpenAI-compatible endpoint. The ones below are spelled correctly; choosing one
                  fills in the two fields under it, and anything else can be typed in.
                </p>

                {PROVIDERS.map((provider) => {
                  const chosen = current?.id === provider.id;
                  const ready = providerReady(provider, Boolean(settings?.apiKeySet));
                  return (
                    <button
                      key={provider.id}
                      type="button"
                      className="preset"
                      aria-current={chosen}
                      onClick={() => choose(provider)}
                    >
                      <span className="preset__text">
                        <span className="preset__name">{provider.name}</span>
                        <span className="preset__url">{provider.baseUrl}</span>
                      </span>
                      {/* Only the row that is actually chosen can say
                          anything about the key, because there is one key and
                          it belongs to the endpoint in the field below. Saying
                          "key stored" against six providers the operator has
                          never used, on the strength of a seventh provider's
                          key, is the same sentence repeated until it means
                          nothing. Local endpoints are the exception: wanting no
                          key is a property of the server, not of this setup. */}
                      {provider.local ? (
                        <span className="preset__state" data-ready="true">
                          On this machine
                        </span>
                      ) : (
                        chosen && (
                          <span className="preset__state" data-ready={ready ? "true" : undefined}>
                            {ready ? "Key stored" : "Needs a key"}
                          </span>
                        )
                      )}
                    </button>
                  );
                })}

                <label className="field" style={{ marginTop: "1.1rem" }}>
                  <span className="field__label">Inference endpoint</span>
                  <input
                    className="input input--mono"
                    value={baseUrl}
                    placeholder="https://openrouter.ai/api/v1"
                    onChange={(event) => setBaseUrl(event.target.value)}
                  />
                  <span className="field__hint">
                    Any OpenAI-compatible base URL. Point it at a local server to run without a
                    network.
                  </span>
                </label>

                <label className="field">
                  <span className="field__label">API key</span>
                  <input
                    className="input input--mono"
                    type="password"
                    value={apiKey}
                    placeholder={
                      settings?.apiKeySet ? `Stored ${settings.apiKeyHint}` : "sk-or-v1-…"
                    }
                    autoComplete="off"
                    onChange={(event) => setApiKey(event.target.value)}
                  />
                  <span className="field__hint">
                    Stored on this machine in a file only your user account can read, and never sent
                    to the webview. Leave blank to keep the current key. A server on this machine
                    usually wants none.
                  </span>
                </label>

                <label className="field">
                  <span className="field__label">Default model</span>
                  <input
                    className="input input--mono"
                    value={model}
                    placeholder="anthropic/claude-sonnet-4.5"
                    onChange={(event) => setModel(event.target.value)}
                  />
                  <span className="field__hint">
                    Used for new agents. Each agent, and each group, can override it.
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
                    Seconds to wait for a model call. A slow local model wants more than a hosted
                    one; a run that hangs wants less. Blank keeps what is stored.
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
                  a conversation ends, and every agent is told why when it hits one.
                </p>

                {LIMITS.map((field) => (
                  <label className="field" key={field.key}>
                    <span className="field__label">{field.label}</span>
                    <input
                      className="input input--mono input--number"
                      type="number"
                      min={field.min}
                      max={field.max}
                      value={limits[field.key]}
                      onChange={(event) =>
                        setLimits((current) => ({
                          ...current,
                          [field.key]: Number(event.target.value) || field.min,
                        }))
                      }
                    />
                    <span className="field__hint">{field.hint}</span>
                  </label>
                ))}
              </>
            )}

            {section === "machines" && (
              <>
                <h3 className="settings__title">Machines</h3>
                <p className="settings__lede">
                  A sandbox each: a desktop and a terminal an agent can actually work in.
                </p>

                <label className="field">
                  <span className="field__label">E2B API key</span>
                  <input
                    className="input input--mono"
                    type="password"
                    value={e2bKey}
                    placeholder={
                      settings?.e2bKeySet ? `Stored ${settings.e2bKeyHint}` : "e2b_… (optional)"
                    }
                    autoComplete="off"
                    onChange={(event) => setE2bKey(event.target.value)}
                  />
                  <span className="field__hint">
                    Gives every agent its own computer: a desktop and a terminal in a sandbox, shown
                    in the corner of its channel. Without a key that pane stays closed.
                  </span>
                </label>

                <label className="field">
                  <span className="field__label">Sleep computers after</span>
                  <input
                    className="input input--mono"
                    inputMode="numeric"
                    value={idleMinutes}
                    placeholder={`${settings?.computerIdleMinutes ?? 15} minutes`}
                    onChange={(event) => setIdleMinutes(event.target.value.replace(/[^0-9]/g, ""))}
                  />
                  <span className="field__hint">
                    Idle minutes before a machine sleeps. Sleeping keeps its disk, so a browser
                    stays signed in and wakes where it left off. Only the running time is billed.
                  </span>
                </label>

                <label className="field">
                  <span className="field__label">Kernel API key</span>
                  <input
                    className="input input--mono"
                    type="password"
                    value={kernelKey}
                    placeholder={
                      settings?.kernelKeySet
                        ? `Stored ${settings.kernelKeyHint}`
                        : "sk_… (optional)"
                    }
                    autoComplete="off"
                    onChange={(event) => setKernelKey(event.target.value)}
                  />
                  <span className="field__hint">
                    Gives every agent its own browser: a Chrome in the cloud, separate from its
                    computer. This is what agents use for the web, because it tells them where
                    everything on a page is instead of making them aim at pixels. Without a key that
                    pane stays closed and agents have no `browse`.
                  </span>
                </label>

                <label className="field">
                  <span className="field__label">Close browsers after</span>
                  <input
                    className="input input--mono"
                    inputMode="numeric"
                    value={browserIdleMinutes}
                    placeholder={`${settings?.browserIdleMinutes ?? 60} minutes`}
                    onChange={(event) =>
                      setBrowserIdleMinutes(event.target.value.replace(/[^0-9]/g, ""))
                    }
                  />
                  <span className="field__hint">
                    Idle minutes before a browser is thrown away. It stops billing within seconds of
                    going quiet either way, and what it was signed in to is kept, so the next one
                    opens signed in to the same accounts. Longer only saves the seconds a fresh one
                    takes to start.
                  </span>
                </label>

                <label className="field field--row">
                  <input
                    type="checkbox"
                    checked={stealth}
                    onChange={(event) => setStealth(event.target.checked)}
                  />
                  <span>
                    <span className="field__label">Hide that browsers are automated</span>
                    <span className="field__hint">
                      For sites that block automation. Kernel disguises the browser and solves
                      captchas. Costs more per browser and needs a plan that includes it, so leave
                      it off until a site turns an agent away.
                    </span>
                  </span>
                </label>
              </>
            )}

            {section === "appearance" && (
              <>
                <h3 className="settings__title">Appearance</h3>
                <p className="settings__lede">
                  Applied as you choose, and kept on this machine. Neither of these is sent anywhere
                  or known to an agent.
                </p>

                <div className="field">
                  <span className="field__label">Reading surface</span>
                  <div className="choices">
                    {(Object.keys(SURFACE_LABELS) as SurfaceMode[]).map((mode) => (
                      <button
                        key={mode}
                        type="button"
                        className="choice"
                        aria-label={`Reading surface: ${SURFACE_LABELS[mode]}`}
                        aria-pressed={prefs.surface === mode}
                        onClick={() => {
                          setPrefs({ surface: mode });
                          applyAppearance(prefs.uiScale, mode);
                        }}
                      >
                        {SURFACE_LABELS[mode]}
                      </button>
                    ))}
                  </div>
                  <span className="field__hint">
                    The column you read in. The rail stays dark either way: it is what makes the
                    column read as a surface rather than as another panel. Currently drawing{" "}
                    {resolveSurface(prefs.surface) === "dark" ? "ink" : "paper"}.
                  </span>
                </div>

                <div className="field">
                  <span className="field__label">Interface scale</span>
                  <div className="choices">
                    {UI_SCALES.map((scale) => (
                      <button
                        key={scale}
                        type="button"
                        className="choice"
                        aria-label={`Interface scale: ${scale}%`}
                        aria-pressed={prefs.uiScale === scale}
                        onClick={() => {
                          setPrefs({ uiScale: scale });
                          applyAppearance(scale, prefs.surface);
                        }}
                      >
                        {scale}%
                      </button>
                    ))}
                  </div>
                  <span className="field__hint">
                    Everything, not just the type: the rail, the rows and the spacing scale with it.
                    The rail and the inspector stop growing before they would crowd out the reading
                    column in a small window.
                  </span>
                </div>
              </>
            )}

            {section === "notifications" && (
              <>
                <h3 className="settings__title">Notifications</h3>
                <p className="settings__lede">
                  Guaca keeps working while you are elsewhere, which is the only time any of these
                  fire. Nothing interrupts you for something already on screen in front of you.
                </p>

                <div className="switch-row">
                  <span className="switch-row__text">
                    <span className="switch-row__label">Notify me at all</span>
                    <span className="switch-row__hint">
                      Off means none of the below, whatever they say. The first notification asks
                      the operating system for permission.
                    </span>
                  </span>
                  <button
                    type="button"
                    className="choice"
                    role="switch"
                    aria-checked={prefs.notify.on}
                    aria-label="Notify me at all"
                    onClick={() => setPrefs({ notify: { ...prefs.notify, on: !prefs.notify.on } })}
                  >
                    {prefs.notify.on ? "On" : "Off"}
                  </button>
                </div>

                {NOTIFY_KINDS.map((kind) => (
                  <div
                    className="switch-row"
                    key={kind}
                    data-off={prefs.notify.on ? undefined : "true"}
                  >
                    <span className="switch-row__text">
                      <span className="switch-row__label">{NOTIFY_COPY[kind].label}</span>
                      <span className="switch-row__hint">{NOTIFY_COPY[kind].hint}</span>
                    </span>
                    <button
                      type="button"
                      className="choice"
                      role="switch"
                      disabled={!prefs.notify.on}
                      // The kind's own setting, not the master switch and the
                      // kind together: the row is already disabled, and what it
                      // has to report is what will apply when the master goes
                      // back on. Reading them together had the label say On
                      // while the control reported itself unchecked.
                      aria-checked={prefs.notify.kinds[kind]}
                      aria-label={NOTIFY_COPY[kind].label}
                      onClick={() =>
                        setPrefs({
                          notify: {
                            ...prefs.notify,
                            kinds: { ...prefs.notify.kinds, [kind]: !prefs.notify.kinds[kind] },
                          },
                        })
                      }
                    >
                      {prefs.notify.kinds[kind] ? "On" : "Off"}
                    </button>
                  </div>
                ))}

                <div style={{ marginTop: "1rem" }}>
                  <button
                    type="button"
                    className="btn"
                    onClick={async () => {
                      const { notifyOperator } = await import("../lib/ipc");
                      const sent = await notifyOperator(
                        "Guaca",
                        "This is what a notification looks like.",
                      );
                      // Deliberately not "it worked". On desktop there is no
                      // per-app grant for the plugin to read, so a machine with
                      // notifications switched off accepts this and shows
                      // nothing: claiming success would be the one message that
                      // cannot be checked. The refusal branch is reachable on
                      // the platforms that do answer the question.
                      setStatus(
                        sent
                          ? {
                              tone: "info",
                              text: "Handed to the operating system. If nothing appeared, Guaca is not allowed to notify you: check Notifications in System Settings.",
                            }
                          : {
                              tone: "error",
                              text: "This machine refused outright. Allow Guaca to notify you in System Settings, then try again.",
                            },
                      );
                    }}
                  >
                    Send a test notification
                  </button>
                  <p className="hint" style={{ marginTop: "0.4rem" }}>
                    The only way to find out whether this machine will show them.
                  </p>
                </div>
              </>
            )}

            {section === "shortcuts" && (
              <>
                <h3 className="settings__title">Shortcuts</h3>
                <p className="settings__lede">
                  Every key the app answers to. The three under Anywhere work wherever you are; the
                  rest belong to the surface they are listed under.
                </p>

                <div className="keys">
                  {SURFACES.map((where) => {
                    const rows = BINDINGS.filter((binding) => binding.where === where);
                    if (rows.length === 0) return null;
                    return (
                      <div key={where}>
                        <p className="keys__group">{where}</p>
                        {rows.map((binding) => (
                          <div className="keys__row" key={binding.id}>
                            <span className="keys__what">{binding.what}</span>
                            <span className="keys__combo">{comboLabel(binding)}</span>
                          </div>
                        ))}
                      </div>
                    );
                  })}
                </div>
              </>
            )}

            {section === "about" && (
              <>
                <div className="about">
                  <h3 className="about__name">Guaca</h3>
                  <p className="about__version">{version ? `Version ${version}` : "Version —"}</p>
                </div>

                <div className="about__facts">
                  <div className="field">
                    <span className="field__label">What this is</span>
                    <span className="field__hint">
                      A local desktop app where you talk to LLM agents and they talk to each other.
                      The agents run here, in this app, not on anybody's server.
                    </span>
                  </div>

                  <div className="field">
                    <span className="field__label">Where it keeps things</span>
                    <span className="field__hint">
                      Everything is under this app's data directory: <code>guac.db</code> for the
                      transcripts, <code>config.json</code> for the settings and your keys, written
                      so only your user account can read it, <code>workspace/</code> for what each
                      agent remembers, and <code>files/</code> for anything attached.
                    </span>
                  </div>

                  <div className="field">
                    <span className="field__label">Licence</span>
                    <span className="field__hint">
                      GNU Affero General Public License v3 or later.
                    </span>
                  </div>
                </div>
              </>
            )}
          </div>
        </div>

        {status && (
          <div
            className={
              status.tone === "error"
                ? "banner banner--error"
                : status.tone === "ok"
                  ? "banner banner--ok"
                  : "banner"
            }
            style={{ margin: "0.75rem 1.35rem 0" }}
          >
            <span>{status.text}</span>
          </div>
        )}

        <div className="settings__foot">
          <button type="button" className="btn" onClick={onClose}>
            Close
          </button>
          <button
            type="button"
            className="btn btn--primary"
            disabled={busy}
            onClick={() => void save()}
          >
            Save
          </button>
        </div>
      </div>
    </div>
  );
}
