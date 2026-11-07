import { useEffect, useId, useRef, useState } from "react";
import { hostPlace } from "../lib/host";
import { api, openExternal } from "../lib/ipc";
import { PROVIDERS, planLabel, providerFor } from "../lib/providers";
import { useStore } from "../lib/store";
import {
  type ClaudeProgram,
  type DeviceCode,
  errorMessage,
  type Pickup,
  type Provider,
  type SettingsPatch,
  type SubscriptionStatus,
} from "../lib/types";
import { ClaudeSignin } from "./ClaudeSignin";
import { SubscriptionModel } from "./ProviderFields";

interface Props {
  onConnected: () => void;
  onBack: () => void;
}

function endpoint(value: string): string {
  return value
    .trim()
    .replace(/\/+$/, "")
    .replace(/\/chat\/completions$/, "")
    .replace(/\/+$/, "");
}

export function OnboardingProvider({ onConnected, onBack }: Props) {
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const capabilities = useStore((s) => s.capabilities);
  const [provider, setProvider] = useState<Provider>(settings?.provider ?? "compatible");
  const [baseUrl, setBaseUrl] = useState(settings?.baseUrl ?? PROVIDERS[0]!.baseUrl);
  const [model, setModel] = useState(settings?.defaultModel ?? PROVIDERS[0]!.model);
  const [subscriptionModel, setSubscriptionModel] = useState(settings?.subscriptionModel ?? "");
  const [effort, setEffort] = useState(settings?.reasoningEffort ?? "auto");
  const [apiKey, setApiKey] = useState("");
  const [subscription, setSubscription] = useState<SubscriptionStatus | null>(null);
  const [pendingCode, setPendingCode] = useState<DeviceCode | null>(null);
  // Whether the host has the `claude` program, which is a fact about the host
  // and not about this screen: undefined while it is being asked, null when the
  // answer could not be read. A screen that guessed "not installed" from a
  // failed read would refuse a provider that works.
  const [claude, setClaude] = useState<ClaudeProgram | null | undefined>(undefined);
  // What this Mac already has, and whether the operator has said no to it.
  // Held here rather than in `Onboarding`, because the offer belongs to the
  // step that would otherwise ask for exactly the same thing.
  const [pickups, setPickups] = useState<Pickup[]>([]);
  const [skipped, setSkipped] = useState(false);
  const [busy, setBusy] = useState<"signin" | "testing" | "saving" | null>(null);
  const [error, setError] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const blockedHint = useId();
  const attempt = useRef(0);

  useEffect(() => {
    let live = true;
    void api.claudeProgram().then(
      (value) => {
        if (live) setClaude(value);
      },
      () => {
        if (live) setClaude(null);
      },
    );
    return () => {
      live = false;
    };
  }, []);

  useEffect(() => {
    const current = attempt.current;
    void api.subscriptionStatus().then(
      (value) => {
        if (attempt.current === current) setSubscription(value);
      },
      () => {
        if (attempt.current === current) setSubscription(null);
      },
    );
    return () => {
      attempt.current += 1;
    };
  }, []);

  // The offer is for a machine the operator is at, so both facts are read
  // rather than inferred: the window has to be the desktop showing this Mac,
  // and the host has to say it is on the operator's own machine. A browser
  // cannot see the host's programs and a box may be shared, so either answer
  // alone would be a guess about whose account that sign-in is.
  const offerHere = capabilities.borrowedSignins && hostPlace() === "thisMac";
  const unconfigured = provider === "compatible" && !settings?.apiKeySet;

  useEffect(() => {
    if (!offerHere || !unconfigured) return;
    let live = true;
    void api.existingSignins().then(
      (found) => {
        if (live) setPickups(found);
      },
      () => {
        // A host that cannot answer has nothing to offer. The provider cards
        // are one click away, so a failed question costs nobody a choice.
        if (live) setPickups([]);
      },
    );
    return () => {
      live = false;
    };
  }, [offerHere, unconfigured]);

  const preset = providerFor(baseUrl);
  const savedKey = Boolean(settings?.apiKeySet && endpoint(baseUrl) === endpoint(settings.baseUrl));
  const needsKey = preset && !preset.local && !savedKey && !apiKey.trim();
  const ineligible = subscription?.signedIn && !subscription.includesCodex;
  const claudeReady = capabilities.claudeProvider && claude?.installed !== false;
  // Detection filled a list and changed nothing. This is the list deciding
  // whether there is an offer to draw, and the operator's Skip is what removes
  // it: no setting is written until one of its buttons is pressed.
  //
  // It may replace the cards only while they are still untouched. A program can
  // take a second to answer, and swapping the screen out from under somebody
  // who has already picked a card and started typing a key is worse than never
  // offering at all.
  const touched =
    provider !== "compatible" ||
    Boolean(apiKey.trim()) ||
    advanced ||
    baseUrl !== (settings?.baseUrl ?? PROVIDERS[0]!.baseUrl);
  const offer = offerHere && unconfigured && !skipped && !touched && pickups.length > 0;

  // The Claude plan is offered beside the other two rather than behind
  // "Advanced", because the people most likely to try this app are already
  // carrying one and a disclosure triangle reads as "for experts". A host that
  // cannot run the program is a reason on the card, never a reason to hide it.
  const claudeBlurb = !capabilities.claudeProvider
    ? "This host does not offer the claude program, so a plan cannot pay for turns here."
    : claude?.installed === false
      ? `Runs the claude program, which this host does not have. Install it there with ${claude.install}, sign in, then come back.`
      : "Use a Claude plan through the claude program on your Guaca host. No API key needed.";

  // Why the primary action cannot be pressed, in the row beside it. The brief
  // step does the same thing for its Continue, and for the same reason: a dead
  // button with no sentence is the first thing this app does to a stranger.
  const blockedReason = !settings
    ? "Waiting for this host to answer with its settings."
    : provider === "compatible" && needsKey
      ? `Paste a key for ${preset?.name ?? "this endpoint"}, then test it.`
      : provider === "claude" && !claudeReady
        ? "This host cannot run the claude program. Choose another card."
        : "";

  const choose = (next: Provider) => {
    setProvider(next);
    setError("");
  };

  const changeEndpoint = (value: string) => {
    setBaseUrl(value);
    setApiKey("");
    setError("");
  };

  const abandon = () => {
    attempt.current += 1;
    setPendingCode(null);
    setBusy(null);
    setError("");
  };

  /**
   * Guaca's own ChatGPT sign-in, from whichever card asked for it.
   *
   * Shared rather than written twice because the browser step is the part a
   * person has to finish, and two copies of it would eventually disagree about
   * what cancelling means. `live` is the caller's answer to "am I still the
   * current attempt", so an abandoned dialog writes nothing.
   */
  const signInChatgpt = async (live: () => boolean): Promise<SubscriptionStatus | null> => {
    if (subscription?.signedIn && subscription.includesCodex) return subscription;
    setBusy("signin");
    const code = await api.beginSubscriptionSignin();
    if (!live()) return null;
    setPendingCode(code);
    void openExternal(code.verificationUrl).catch(() => {});
    const account = await api.completeSubscriptionSignin(code);
    if (!live()) return null;
    setPendingCode(null);
    setSubscription(account);
    return account;
  };

  /**
   * Takes one sign-in and makes it the provider.
   *
   * Claude is a setting because that program already holds the sign-in: the
   * `claude` provider is only "run that program", so accepting is the whole of
   * it, and the test is the same one the card runs rather than a second way in.
   *
   * Codex cannot be a setting, and this is the trade-off `pickup.rs` argues: the
   * token under `~/.codex` belongs to that CLI and rotates on refresh, so
   * copying it would leave the two programs signing each other out. What is
   * reused instead is the account and the browser session that CLI already
   * opened, through Guaca's own sign-in.
   */
  const accept = async (pickup: Pickup) => {
    if (busy || !settings) return;
    const current = ++attempt.current;
    const live = () => attempt.current === current;
    setError("");
    try {
      if (pickup.program === "codex") {
        const account = await signInChatgpt(live);
        if (!account) return;
        if (!account.signedIn || !account.includesCodex) {
          setError(
            `This ${planLabel(account.plan) || "ChatGPT"} plan does not include Codex. Skip to choose another provider.`,
          );
          return;
        }
        setBusy("saving");
        const next = await api.updateSettings({ provider: "chatgpt" });
        if (!live()) return;
        setSettings(next);
        onConnected();
      } else {
        setBusy("testing");
        await api.testConnection({ provider: "claude" });
        if (!live()) return;
        setBusy("saving");
        const next = await api.updateSettings({ provider: "claude" });
        if (!live()) return;
        setSettings(next);
        onConnected();
      }
    } catch (failure) {
      if (live()) {
        setError(`${errorMessage(failure)} Skip to choose another provider, or try again.`);
      }
    } finally {
      if (live()) {
        setPendingCode(null);
        setBusy(null);
      }
    }
  };

  const connect = async () => {
    if (busy || !settings) return;
    if (provider === "claude" && !claudeReady) return;
    if (provider === "compatible" && (needsKey || !baseUrl.trim() || !model.trim())) {
      setError("Enter an endpoint, a model, and the key for that provider, then try again.");
      return;
    }
    const current = ++attempt.current;
    const live = () => attempt.current === current;
    setError("");
    try {
      if (provider === "chatgpt") {
        const account = await signInChatgpt(live);
        if (!account) return;
        if (!account.signedIn || !account.includesCodex) {
          setError(
            `This ${planLabel(account.plan) || "ChatGPT"} plan does not include Codex. Use an API key or sign in with an eligible account.`,
          );
          return;
        }
      }
      const patch: SettingsPatch =
        provider === "compatible"
          ? {
              provider,
              baseUrl: baseUrl.trim(),
              defaultModel: model.trim(),
              ...(apiKey.trim() ? { apiKey: apiKey.trim() } : savedKey ? {} : { apiKey: "" }),
            }
          : provider === "chatgpt"
            ? {
                provider,
                ...(subscriptionModel.trim()
                  ? { subscriptionModel: subscriptionModel.trim() }
                  : {}),
                reasoningEffort: effort,
              }
            : { provider };
      setBusy("testing");
      await api.testConnection(patch);
      if (!live()) return;
      setBusy("saving");
      const next = await api.updateSettings(patch);
      if (!live()) return;
      setApiKey("");
      setSettings(next);
      onConnected();
    } catch (failure) {
      if (live())
        setError(
          `${errorMessage(failure)} Check your connection or provider settings, then try again.`,
        );
    } finally {
      if (live()) {
        setPendingCode(null);
        setBusy(null);
      }
    }
  };

  // The state of whatever was last pressed, in one place: the provider cards
  // and the offer ask different questions but answer with the same failure,
  // the same wait, and the same sign-in code.
  const notices = (
    <>
      {error && (
        <p className="onboarding-notice" role="alert">
          {error}
        </p>
      )}
      {busy && !pendingCode && (
        <p className="onboarding-notice" role="status">
          {busy === "signin"
            ? "Starting sign-in…"
            : busy === "testing"
              ? "Testing the connection…"
              : "Saving your provider…"}
        </p>
      )}
      {pendingCode && (
        <div className="onboarding-notice" role="status">
          <p>
            Enter this code in your browser: <strong>{pendingCode.userCode}</strong>
          </p>
          <p>{pendingCode.verificationUrl}</p>
          <button
            type="button"
            className="btn"
            onClick={() => {
              const current = attempt.current;
              void openExternal(pendingCode.verificationUrl).catch(() => {
                if (attempt.current === current)
                  setError("The browser could not open. Copy the address above into your browser.");
              });
            }}
          >
            Open sign-in page
          </button>
          <p>
            Only enter this code if you started this sign-in here. If you cancel, do not finish it
            in the browser.
          </p>
        </div>
      )}
    </>
  );

  if (offer)
    return (
      <section className="onboarding-provider" aria-label="Connect a provider">
        <h2>This Mac is already signed in</h2>
        <p>
          Guaca found a sign-in this Mac already has. Use it for your agents' turns, or skip and
          choose a provider yourself. Nothing changes until you choose.
        </p>
        <ul className="onboarding-pickups">
          {pickups.map((pickup) => (
            <li className="onboarding-pickup" key={pickup.program}>
              <div className="onboarding-pickup__head">
                <strong>{pickup.program === "claudeCode" ? "Claude Code" : "Codex"}</strong>
                <span>
                  {pickup.program === "claudeCode"
                    ? [
                        pickup.account || "signed in on this Mac",
                        pickup.plan ? `${planLabel(pickup.plan)} plan` : "",
                      ]
                        .filter(Boolean)
                        .join(" · ")
                    : "signed in to ChatGPT"}
                </span>
              </div>
              {pickup.program === "codex" && (
                <p className="onboarding-pickup__why">
                  Guaca keeps its own ChatGPT sign-in rather than copying Codex's token: both
                  programs refresh that token, and the first to refresh would sign the other one
                  out. This opens the browser you just used, so it is one confirmation.
                </p>
              )}
              <button
                type="button"
                className="btn btn--primary"
                disabled={Boolean(busy)}
                onClick={() => void accept(pickup)}
              >
                {pickup.program === "claudeCode"
                  ? "Use Claude Code"
                  : "Sign in with the same account"}
              </button>
            </li>
          ))}
        </ul>
        {notices}
        <div className="onboarding-actions">
          <button
            type="button"
            className="btn"
            disabled={busy === "saving"}
            onClick={() => {
              abandon();
              setApiKey("");
              onBack();
            }}
          >
            Back
          </button>
          {busy === "signin" ? (
            <button type="button" className="btn" onClick={abandon}>
              Cancel sign-in
            </button>
          ) : (
            <button
              type="button"
              className="btn"
              disabled={Boolean(busy)}
              onClick={() => setSkipped(true)}
            >
              Skip and choose a provider
            </button>
          )}
        </div>
      </section>
    );

  return (
    <section className="onboarding-provider" aria-label="Connect a provider">
      <h2>Choose how your agents are paid for</h2>
      <p>
        Guaca uses your provider account. A subscription uses its included quota; an API key is
        billed separately by the provider.
      </p>
      <div className="onboarding-options onboarding-options--three">
        <button
          type="button"
          className="onboarding-choice"
          aria-pressed={provider === "chatgpt"}
          disabled={Boolean(busy)}
          onClick={() => choose("chatgpt")}
        >
          <strong>ChatGPT subscription</strong>
          <span>Sign in with a plan that includes Codex. No API key needed.</span>
        </button>
        <button
          type="button"
          className="onboarding-choice"
          aria-pressed={provider === "claude"}
          disabled={Boolean(busy) || !claudeReady}
          onClick={() => choose("claude")}
        >
          <strong>Claude subscription</strong>
          <span>{claudeBlurb}</span>
        </button>
        <button
          type="button"
          className="onboarding-choice"
          aria-pressed={provider === "compatible"}
          disabled={Boolean(busy)}
          onClick={() => choose("compatible")}
        >
          <strong>API key</strong>
          <span>Use OpenRouter or OpenAI, with usage billed to that account.</span>
        </button>
      </div>

      {provider === "compatible" && (
        <>
          <label className="field">
            <span className="field__label">API provider</span>
            <select
              className="input"
              value={preset?.id === "openrouter" || preset?.id === "openai" ? preset.id : "custom"}
              disabled={Boolean(busy)}
              onChange={(event) => {
                if (event.target.value === "custom") {
                  setAdvanced(true);
                  changeEndpoint("");
                  return;
                }
                const selected = PROVIDERS.find((entry) => entry.id === event.target.value)!;
                changeEndpoint(selected.baseUrl);
                setModel(selected.model);
              }}
            >
              <option value="openrouter">OpenRouter</option>
              <option value="openai">OpenAI</option>
              <option value="custom">Custom endpoint</option>
            </select>
          </label>
          <label className="field">
            <span className="field__label">API key</span>
            <input
              className="input input--mono"
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={apiKey}
              disabled={Boolean(busy)}
              placeholder={savedKey ? "Saved key will be used" : "Paste your provider's API key"}
              onChange={(event) => setApiKey(event.target.value)}
            />
            <span className="field__hint">
              {savedKey
                ? "Leave blank to keep the saved key for this endpoint."
                : "Stored only on your Guaca host. A saved key from another endpoint is never reused."}
            </span>
          </label>
          <p className="onboarding-notice">
            API usage is separate from a ChatGPT or Claude subscription. Testing makes a small model
            request that may be billed.
          </p>
        </>
      )}

      {provider === "chatgpt" && (
        <div className="onboarding-notice">
          <p>
            Turns use your ChatGPT plan's Codex quota, not API credits. A group can reach that quota
            faster than one person.
          </p>
          {subscription?.signedIn && (
            <p>
              Signed in as {subscription.email || "your ChatGPT account"}
              {subscription.plan ? ` · ${planLabel(subscription.plan)}` : ""}.
            </p>
          )}
          {ineligible && (
            <p role="alert">
              This plan does not include Codex. Use an API key or sign in with an eligible account.
            </p>
          )}
        </div>
      )}

      {provider === "claude" && claudeReady && (
        <div className="onboarding-notice">
          Claude's own account and model settings apply; Guaca passes neither, so a model named here
          is not used. Testing makes a small request using that account.
          <ClaudeSignin />
        </div>
      )}

      <details
        className="onboarding-advanced"
        open={advanced}
        onToggle={(event) => setAdvanced(event.currentTarget.open)}
      >
        <summary>Advanced options</summary>
        <fieldset disabled={Boolean(busy)}>
          {provider === "compatible" && (
            <>
              <label className="field">
                <span className="field__label">Inference endpoint</span>
                <input
                  className="input input--mono"
                  value={baseUrl}
                  onChange={(event) => changeEndpoint(event.target.value)}
                />
              </label>
              <label className="field">
                <span className="field__label">Default model</span>
                <input
                  className="input input--mono"
                  value={model}
                  onChange={(event) => setModel(event.target.value)}
                />
              </label>
              <p className="field__hint">
                Use an OpenAI-compatible endpoint and a model it supports. A local endpoint may not
                need a key.
                {!capabilities.loopbackEndpoints &&
                  " This host cannot reach model endpoints on your own computer."}
              </p>
            </>
          )}
          {advanced && provider === "chatgpt" && subscription?.signedIn && (
            <SubscriptionModel
              value={subscriptionModel}
              models={settings?.subscriptionModels ?? []}
              onChange={setSubscriptionModel}
              effort={effort}
              onEffortChange={(value) => setEffort(value ?? "auto")}
            />
          )}
        </fieldset>
      </details>

      {notices}
      <div className="onboarding-actions">
        <button
          type="button"
          className="btn"
          disabled={busy === "saving"}
          onClick={() => {
            abandon();
            setApiKey("");
            onBack();
          }}
        >
          Back
        </button>
        {busy === "signin" ? (
          <button type="button" className="btn" onClick={abandon}>
            Cancel sign-in
          </button>
        ) : (
          <button
            type="button"
            className="btn btn--primary"
            aria-describedby={blockedReason && !busy ? blockedHint : undefined}
            disabled={
              Boolean(busy) ||
              !settings ||
              Boolean(provider === "compatible" && needsKey) ||
              (provider === "claude" && !claudeReady)
            }
            onClick={() => void connect()}
          >
            {provider === "chatgpt" && (!subscription?.signedIn || ineligible)
              ? "Sign in with ChatGPT"
              : "Test and continue"}
          </button>
        )}
        {blockedReason && !busy && <span id={blockedHint}>{blockedReason}</span>}
      </div>
    </section>
  );
}
