/**
 * Settings, as nine places rather than one scroll.
 *
 * Every piece of state lives here, in the shell, and the panes are given values
 * and setters. That is not tidiness: the shell is unmounted when the dialog
 * closes, so state held by a pane would be discarded every time the operator
 * changed section, and typing an endpoint, glancing at Limits and coming back
 * would silently lose the endpoint.
 *
 * Save stays in the foot, outside the scrolling half, for the same reason it
 * used to be at the bottom of one column: it acts on everything, not on the
 * section that happens to be open. It is drawn only while something is waiting
 * for it, which is what keeps that from being a lie. Five of the nine panes
 * stage nothing at all — Appearance and Notifications write as they are
 * clicked, Account acts at once, Shortcuts and About are read-only — so a Save
 * pinned there regardless spent most of its life offering to save settings that
 * were already saved, which reads as the opposite. Test is in the Provider pane
 * because it reads that pane and acts on none of the others, and it deliberately
 * sends what is on screen rather than what is stored. Save does not close.
 */

import { useEffect, useRef, useState } from "react";

import { applyAppearance, resolveSurface } from "../lib/appearance";
import { api, openExternal } from "../lib/ipc";
import { BINDINGS, comboLabel, SURFACES } from "../lib/keybinds";
import { LIMITS } from "../lib/limits";
import { NOTIFY_KINDS, type NotifyKind, type SurfaceMode, UI_SCALES } from "../lib/prefs";
import { type Provider as Preset, planLabel } from "../lib/providers";
import { useStore } from "../lib/store";
import {
  type AccountConnectors,
  type AccountStatus,
  type DeviceCode,
  errorMessage,
  type GuardLimits,
  type Provider as ProviderKind,
  type SettingsPatch,
  type SubscriptionStatus,
} from "../lib/types";
import { ProviderPresets, SubscriptionModel } from "./ProviderFields";

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
  "account",
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
  account: "Account",
  appearance: "Appearance",
  notifications: "Notifications",
  shortcuts: "Shortcuts",
  about: "About",
};

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

/**
 * A duration box, which is blank while it is inheriting what is stored.
 *
 * So an edit is a number that is not the stored one, and retyping what the
 * placeholder already says is not one.
 */
const differs = (text: string, stored: number | undefined) =>
  text.trim() !== "" && Number(text) !== stored;

export function SettingsDialog({ onClose, section: opening }: Props) {
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const prefs = useStore((s) => s.prefs);
  const setPrefs = useStore((s) => s.setPrefs);

  const [section, setSection] = useState<Section>(opening ?? "general");
  const [operatorName, setOperatorName] = useState(settings?.operatorName ?? "");
  const [provider, setProvider] = useState<ProviderKind>(settings?.provider ?? "compatible");
  const [baseUrl, setBaseUrl] = useState(settings?.baseUrl ?? "");
  const [model, setModel] = useState(settings?.defaultModel ?? "");
  const [subscriptionModel, setSubscriptionModel] = useState(settings?.subscriptionModel ?? "");
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

  // The sign-in, which is three states rather than a boolean: not signed in,
  // waiting for the operator to enter a code in a browser, and signed in. All
  // three live here for the reason everything else does — the shell survives a
  // section change and the pane does not, and a sign-in half finished must not
  // be discarded by a glance at Limits.
  const [subscription, setSubscription] = useState<SubscriptionStatus | null>(null);
  const [pendingCode, setPendingCode] = useState<DeviceCode | null>(null);

  // The Guaca account, which is three states for the same reason: signed out,
  // waiting on a browser, and signed in. `connectors` is what the service says
  // the account holds, and it is read rather than kept because it changes when
  // the operator authorizes something in a browser rather than when this app
  // does anything.
  // Whether the `claude` program is on this machine. Read rather than assumed,
  // and read from the command the repository panel already asks the same
  // question with: it is the same binary, and a second way to ask would be a
  // second answer to keep true. Null while it is still being asked, which is
  // not the same as "not installed" and must not draw as it.
  const [claudeInstalled, setClaudeInstalled] = useState<boolean | null>(null);
  const [account, setAccount] = useState<AccountStatus | null>(null);
  const [connectors, setConnectors] = useState<AccountConnectors | null>(null);
  const [linking, setLinking] = useState(false);

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

  // Asked on mount rather than when the section opens: it is a process spawn,
  // it is wanted the moment the provider list draws, and an operator who opens
  // Settings to change their provider should not watch a row resolve under
  // them.
  useEffect(() => {
    let live = true;
    void api
      .codingHarnesses()
      .then((harnesses) => {
        if (live) {
          setClaudeInstalled(harnesses.find((h) => h.harness === "claude")?.installed ?? false);
        }
      })
      .catch(() => {
        // Asked again next time Settings opens. A row that cannot say whether
        // the program is there says nothing, which is what null draws as.
      });
    return () => {
      live = false;
    };
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

  const patch = (): SettingsPatch => ({
    provider,
    baseUrl,
    defaultModel: model,
    // Both models are always sent, whichever provider is chosen. Each belongs
    // to one provider and neither is cleared by the other, so an operator who
    // tries a subscription and goes back finds their endpoint model intact.
    ...(subscriptionModel.trim() ? { subscriptionModel: subscriptionModel.trim() } : {}),
    // Omitted when blank, so saving without retyping keeps the stored key.
    ...(apiKey.trim() ? { apiKey: apiKey.trim() } : {}),
    ...(e2bKey.trim() ? { e2bApiKey: e2bKey.trim() } : {}),
    ...(idleMinutes.trim() ? { computerIdleMinutes: Number(idleMinutes) } : {}),
    ...(kernelKey.trim() ? { kernelApiKey: kernelKey.trim() } : {}),
    ...(browserIdleMinutes.trim() ? { browserIdleMinutes: Number(browserIdleMinutes) } : {}),
    // A checkbox is never blank, so it goes every time. Omitting it would leave
    // the only way to turn stealth back off unreachable.
    browserStealth: stealth,
    ...(timeout.trim() ? { requestTimeoutSecs: Number(timeout) } : {}),
  });

  /**
   * Whether anything on this dialog is waiting to be saved.
   *
   * Read across every pane rather than the open one, because the shell holds
   * all of their state: an endpoint typed on Provider and left there while the
   * operator reads Limits is still unsaved, and a Save that went missing on the
   * way past is how it would get lost.
   *
   * A key is compared with blank rather than with what is stored, because what
   * is stored cannot be read back. A blank box is not an edit, and a box of
   * spaces is not one either: that is exactly what `patch` omits.
   */
  const dirty =
    operatorName !== (settings?.operatorName ?? "") ||
    provider !== (settings?.provider ?? "compatible") ||
    baseUrl !== (settings?.baseUrl ?? "") ||
    model !== (settings?.defaultModel ?? "") ||
    subscriptionModel !== (settings?.subscriptionModel ?? "") ||
    stealth !== (settings?.browserStealth ?? false) ||
    apiKey.trim() !== "" ||
    e2bKey.trim() !== "" ||
    kernelKey.trim() !== "" ||
    differs(idleMinutes, settings?.computerIdleMinutes) ||
    differs(browserIdleMinutes, settings?.browserIdleMinutes) ||
    differs(timeout, settings?.requestTimeoutSecs) ||
    LIMITS.some((field) => limits[field.key] !== settings?.limits[field.key]);

  const save = async () => {
    setBusy(true);
    setStatus(null);
    try {
      const next = await api.updateSettings({ operatorName, limits, ...patch() });
      setSettings(next);
      // Read the limits back rather than leaving what was typed. The `max` on a
      // number input is advisory — a pasted or typed value sails past it — and
      // the runtime sanitizes what it stores, so a relay depth of 40 is kept as
      // 16. Saying "Saved." over a box still reading 40 tells the operator
      // something untrue about what is running.
      setLimits(next.limits);
      // And every box that stages something goes back to blank, which is the
      // same argument one field further. A key box left holding a key the
      // runtime already has is indistinguishable from an edit nobody saved, and
      // a duration box is worse than that: those three are clamped on the way
      // in, so a box still reading 2000 sits under "Saved." claiming a machine
      // sleeps after thirty-three hours when the runtime stored twenty-four.
      // Blank is these boxes' resting state and the placeholder beside each one
      // is read from what came back, so what is on screen after this is what is
      // running.
      setApiKey("");
      setE2bKey("");
      setKernelKey("");
      setIdleMinutes("");
      setBrowserIdleMinutes("");
      setTimeoutSecs("");
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

  const choose = (preset: Preset) => {
    // Choosing an endpoint is also choosing to pay with a key. Leaving the
    // provider on the subscription would have the operator fill in a URL and a
    // key that nothing used, with no error to explain why.
    setProvider("compatible");
    setBaseUrl(preset.baseUrl);
    if (preset.model) setModel(preset.model);
  };

  // Read when the pane that shows it is opened, not at startup: nothing else in
  // the app needs to know, and a status nobody is looking at is a round trip
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
        // A status that cannot be read is drawn as not signed in, which is what
        // it is as far as anything here can act on.
        if (live) setSubscription({ signedIn: false, email: "", plan: "", includesCodex: false });
      });
    return () => {
      live = false;
    };
  }, [section, subscription]);

  // Same rule as above: read when the pane is opened. An install that never
  // opens this pane never asks the service anything, which is the whole point
  // of the account being optional.
  useEffect(() => {
    if (section !== "account" || account) return;
    let live = true;
    void api
      .accountStatus()
      .then((value) => {
        if (live) setAccount(value);
      })
      .catch(() => {
        if (live) setAccount({ signedIn: false, email: "", origin: "" });
      });
    return () => {
      live = false;
    };
  }, [section, account]);

  // What the account holds, once there is one. Separate from the status because
  // it is a network call to the service and the status is a local read, so a
  // service that is slow or down still draws the account correctly.
  useEffect(() => {
    if (section !== "account" || !account?.signedIn || connectors) return;
    let live = true;
    void api
      .accountConnectors()
      .then((value) => {
        if (live) setConnectors(value);
      })
      .catch((error) => {
        // A sign-in the service no longer recognizes comes back as `signedOut`,
        // and the honest thing to draw is a signed-out pane rather than an
        // account with an empty list under it.
        if (!live) return;
        if ((error as { kind?: string })?.kind === "signedOut") {
          setAccount({ signedIn: false, email: "", origin: account.origin });
        }
      });
    return () => {
      live = false;
    };
  }, [section, account, connectors]);

  /**
   * The whole account sign-in, in one call.
   *
   * A browser opens and the answer comes back to a port Rust bound before it
   * asked, so there is nothing for the operator to carry and nothing to draw in
   * between. Closing the dialog abandons it and leaves nothing behind.
   */
  const linkAccount = async () => {
    setStatus(null);
    setLinking(true);
    try {
      const next = await api.signInAccount();
      setAccount(next);
      setConnectors(null);
      setStatus({ tone: "ok", text: `Signed in as ${next.email || "your Guaca account"}.` });
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setLinking(false);
    }
  };

  const unlinkAccount = async () => {
    setLinking(true);
    setStatus(null);
    try {
      const next = await api.signOutAccount();
      setAccount(next);
      setConnectors(null);
      setStatus({ tone: "ok", text: "Signed out." });
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setLinking(false);
    }
  };

  /**
   * Starts the sign-in and then waits for it, in one action.
   *
   * The wait is the whole point of the two-command split: the code is drawn as
   * soon as the first call returns, and the second is left parked for as long as
   * the operator takes. Closing the dialog abandons it and leaves nothing behind.
   */
  const signIn = async () => {
    setStatus(null);
    setBusy(true);
    let code: DeviceCode;
    try {
      code = await api.beginSubscriptionSignin();
      setPendingCode(code);
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
      setBusy(false);
      return;
    }

    // Opened for the operator rather than left as a link to find. It goes to
    // the system browser: the sign-in belongs to a ChatGPT session this webview
    // has no business holding.
    void openExternal(code.verificationUrl).catch(() => {
      // The URL is on screen beside the code, so a browser that will not open
      // is a copy-and-paste rather than a dead end.
    });

    try {
      const next = await api.completeSubscriptionSignin(code);
      setSubscription(next);
      setPendingCode(null);
      // Chosen on success rather than offered as a further step: nobody signs
      // in to a subscription in order to keep paying with a key.
      setProvider("chatgpt");
      setStatus({
        tone: next.includesCodex ? "ok" : "error",
        text: next.includesCodex
          ? // The second sentence only where there is a Save to press. An
            // operator whose sign-in expired is already on the subscription, so
            // signing back in leaves nothing waiting to be saved and the button
            // is not drawn: sending them to it would be sending them nowhere.
            `Signed in as ${next.email || "your ChatGPT account"}.${
              settings?.provider === "chatgpt" ? "" : " Save to start using it."
            }`
          : `Signed in as ${next.email || "your ChatGPT account"}, but a ${planLabel(next.plan)} plan does not include Codex. Use an API key instead.`,
      });
    } catch (error) {
      setPendingCode(null);
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setBusy(false);
    }
  };

  const signOut = async () => {
    setBusy(true);
    setStatus(null);
    try {
      const next = await api.signOutSubscription();
      setSettings(next);
      setProvider(next.provider);
      setSubscription({ signedIn: false, email: "", plan: "", includesCodex: false });
      setStatus({ tone: "ok", text: "Signed out." });
    } catch (error) {
      setStatus({ tone: "error", text: errorMessage(error) });
    } finally {
      setBusy(false);
    }
  };

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
                  Three ways to pay for a turn: a subscription you sign in to, a program already
                  signed in on this machine, or an endpoint and a key you paste. What is chosen here
                  is the default. Any group can pay its own way, and one that does is not affected
                  by anything on this page.
                </p>

                {/* Its own block, above the endpoint list, because it is not an
                    endpoint. Nothing here is typed: there is no URL to get
                    wrong and no key to paste, which is most of what the list
                    below exists to protect against. */}
                <div className="preset preset--plain" aria-current={provider === "chatgpt"}>
                  <span className="preset__text">
                    <span className="preset__name">ChatGPT subscription</span>
                    <span className="preset__url">
                      {subscription?.signedIn
                        ? `${subscription.email || "signed in"}${
                            subscription.plan ? ` · ${planLabel(subscription.plan)}` : ""
                          }`
                        : "Sign in and your plan pays for turns, with no per-token bill"}
                    </span>
                  </span>
                  {subscription?.signedIn ? (
                    <span className="preset__actions">
                      {provider === "chatgpt" ? (
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
                      )}
                      <button
                        type="button"
                        className="btn btn--small"
                        disabled={busy}
                        onClick={() => void signOut()}
                      >
                        Sign out
                      </button>
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="btn btn--small"
                      disabled={busy || pendingCode !== null}
                      onClick={() => void signIn()}
                    >
                      Sign in
                    </button>
                  )}
                </div>

                {/* Shown for as long as the sign-in is parked. The code is the
                    thing the operator has to carry, so it is the largest thing
                    here, and the URL is beside it because a browser that did
                    not open leaves them nothing else to go on. */}
                {pendingCode && (
                  <div className="devicecode" role="status">
                    <p className="devicecode__lede">
                      Enter this code in the browser window that just opened. It expires in fifteen
                      minutes.
                    </p>
                    <p className="devicecode__code">{pendingCode.userCode}</p>
                    <p className="devicecode__url">{pendingCode.verificationUrl}</p>
                    <p className="hint">
                      Only continue if you started this sign-in here. If anything else gave you this
                      code, cancel it.
                    </p>
                  </div>
                )}

                {/* No sign-in, no key and no model. All three belong to the
                    program, which is why this row has one control on it: the
                    only thing this app decides is whether to use it. */}
                <div className="preset preset--plain" aria-current={provider === "claude"}>
                  <span className="preset__text">
                    <span className="preset__name">Claude subscription</span>
                    <span className="preset__url">
                      {claudeInstalled === false
                        ? "Runs the claude program, which is not installed on this machine"
                        : "Runs the claude program, and uses whatever it is signed in to and set to"}
                    </span>
                  </span>
                  {claudeInstalled === false ? (
                    <span className="preset__state" data-ready="false">
                      Not installed
                    </span>
                  ) : provider === "claude" ? (
                    <span className="preset__state" data-ready="true">
                      In use
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="btn btn--small"
                      aria-label="Use the Claude subscription"
                      disabled={busy || claudeInstalled === null}
                      onClick={() => setProvider("claude")}
                    >
                      Use it
                    </button>
                  )}
                </div>

                {provider === "claude" && (
                  <p className="hint">
                    Which model runs, and which account pays, are Claude's own settings. This app
                    passes neither, so a model named here or on an agent is not used while Claude is
                    the provider. Your plan has a rolling quota rather than a per-token bill, and a
                    crew reaches it faster than one person does.
                  </p>
                )}

                {subscription?.signedIn && provider === "chatgpt" && (
                  <SubscriptionModel
                    value={subscriptionModel}
                    models={settings?.subscriptionModels ?? []}
                    onChange={setSubscriptionModel}
                    hint="The default model for any group that does not name one, and any agent that does not name one. A subscription has an hourly quota rather than a per-token bill, so a crew that talks a lot reaches the ceiling faster than one person would."
                  />
                )}

                <p className="settings__lede" style={{ marginTop: "1.4rem" }}>
                  Or any OpenAI-compatible endpoint. The ones below are spelled correctly; choosing
                  one fills in the two fields under it, and anything else can be typed in.
                </p>

                <ProviderPresets
                  baseUrl={baseUrl}
                  active={provider === "compatible"}
                  keySet={Boolean(settings?.apiKeySet)}
                  onChoose={choose}
                />

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
                    The default model for any group that does not name one, and any agent that does
                    not name one.
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
                  a conversation ends, and every agent is told why when it hits one. A group can set
                  its own; these are what it falls back to.
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

            {section === "account" && (
              <>
                <h3 className="settings__title">Account</h3>
                <p className="settings__lede">
                  Optional, and it stays that way. Guaca runs on this machine with your own keys,
                  and everything it does today it does without an account. Signing in adds one
                  thing: guaca.bot holds an OAuth client, so an agent can reach a service that only
                  issues access to a registered application. Gmail is the example. Guaca cannot be
                  that application, because its client secret would be inside a download anybody can
                  read.
                </p>

                <div className="preset preset--plain" aria-current={account?.signedIn === true}>
                  <span className="preset__text">
                    <span className="preset__name">Guaca account</span>
                    <span className="preset__url">
                      {account?.signedIn
                        ? account.email || "signed in"
                        : "Sign in to authorize Gmail, Drive or a GitHub repository for your agents"}
                    </span>
                  </span>
                  {account?.signedIn ? (
                    <span className="preset__actions">
                      <button
                        type="button"
                        className="btn btn--small"
                        disabled={linking}
                        onClick={() => void openExternal(`${account.origin}/app`)}
                      >
                        Manage
                      </button>
                      <button
                        type="button"
                        className="btn btn--small"
                        disabled={linking}
                        onClick={() => void unlinkAccount()}
                      >
                        Sign out
                      </button>
                    </span>
                  ) : (
                    <button
                      type="button"
                      className="btn btn--small"
                      disabled={linking}
                      onClick={() => void linkAccount()}
                    >
                      Sign in
                    </button>
                  )}
                </div>

                {/* Shown for as long as the browser has it. There is no code to
                    carry here, so this says what is happening rather than
                    giving the operator something to do. */}
                {linking && (
                  <div className="devicecode" role="status">
                    <p className="devicecode__lede">
                      Finish in the browser window that just opened. Guaca is listening on a port on
                      this machine for the answer, and gives up after five minutes.
                    </p>
                    <p className="hint">
                      Only continue if you started this here. Nothing else can have opened it.
                    </p>
                  </div>
                )}

                {account?.signedIn && (
                  <>
                    <p className="settings__lede" style={{ marginTop: "1.4rem" }}>
                      What this account has authorized. Ticking and unticking happens on guaca.bot,
                      in a browser, because that is where the consent screens are.
                    </p>
                    {connectors ? (
                      <ul className="settings__list">
                        {connectors.providers.map((provider) => {
                          const granted = provider.capabilities.filter((c) => c.granted);
                          return (
                            <li key={provider.id}>
                              <span className="field__label">{provider.label}</span>
                              <span className="field__hint">
                                {granted.length > 0
                                  ? granted.map((c) => c.label).join(", ")
                                  : "Nothing authorized yet."}
                              </span>
                            </li>
                          );
                        })}
                        {connectors.providers.length === 0 && (
                          <li>
                            <span className="field__hint">
                              This deployment offers no connectors.
                            </span>
                          </li>
                        )}
                      </ul>
                    ) : (
                      <p className="field__hint">Asking the service&hellip;</p>
                    )}
                  </>
                )}

                <p className="field__hint" style={{ marginTop: "1.4rem" }}>
                  What leaves this machine, and only once you have signed in: a request for an
                  access token when an agent uses a connector you authorized. Your conversations,
                  your agents and your keys are not sent anywhere, with or without an account.
                  Signing out here forgets the sign-in on this machine immediately.
                </p>

                {account && account.origin !== "https://guaca.bot" && account.origin !== "" && (
                  /* Development, or a self-hosted service. Worth saying out loud
                     rather than leaving an operator to assume guaca.bot: the two
                     hold different accounts and only one of them is the real one. */
                  <p className="field__hint">
                    This build signs in to <code>{account.origin}</code>, not guaca.bot.
                  </p>
                )}
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
                    <span className="field__label">License</span>
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
          {dirty && (
            <button
              type="button"
              className="btn btn--primary"
              disabled={busy}
              onClick={() => void save()}
            >
              Save
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
