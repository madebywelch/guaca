import { useEffect, useId, useRef, useState } from "react";
import { hostPlace } from "../lib/host";
import { api, openExternal } from "../lib/ipc";
import { type HeldProvider, waitsForHost } from "../lib/onboarding";
import { PROVIDERS, planLabel, providerFor } from "../lib/providers";
import { useStore } from "../lib/store";
import { isUnreachable } from "../lib/transport";
import {
  type ClaudeProgram,
  type DeviceCode,
  errorMessage,
  isCommandError,
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
  waiting?: boolean;
  initial?: HeldProvider | null;
  failure?: unknown;
  onDraft?: (fields: HeldProvider) => void;
}

type Request = { kind: "idle" } | { kind: "running" };
type RequestEvent = "start" | "finish";

export function transitionRequest(state: Request, event: RequestEvent): Request {
  if (event === "finish") return { kind: "idle" };
  return state.kind === "idle" ? { kind: "running" } : state;
}

function endpoint(value: string): string {
  return value
    .trim()
    .replace(/\/+$/, "")
    .replace(/\/chat\/completions$/, "")
    .replace(/\/+$/, "");
}

export function explainFailure(
  failure: unknown,
  { name, address, savedOnly }: { name: string; address: string; savedOnly: boolean },
): { keyError: string; error: string } {
  if (isCommandError(failure) && failure.kind === "keyRejected")
    return {
      keyError: savedOnly
        ? `${name} did not accept the saved key. Paste a new key, then test again.`
        : `${name} did not accept that key. Check that you pasted all of it, or create a new key there, then test again.`,
      error: "",
    };
  if (isCommandError(failure) && failure.kind === "endpointUnreachable")
    return {
      keyError: "",
      error: `Guaca could not reach ${address}. Check that the server there is running and the address is right, then test again.`,
    };
  if (isUnreachable(failure))
    return {
      keyError: "",
      error:
        "Guaca could not reach its host, so nothing was tested. Check that the host is running, then try again.",
    };
  return {
    keyError: "",
    error: `The test did not pass: ${errorMessage(failure).replace(/[.\s]+$/, "")}. Check the endpoint, the key and the model above, then try again.`,
  };
}

export function OnboardingProvider({
  onConnected,
  onBack,
  waiting = false,
  initial = null,
  failure,
  onDraft,
}: Props) {
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const capabilities = useStore((s) => s.capabilities);
  const [provider, setProvider] = useState<Provider>(
    initial?.provider ?? settings?.provider ?? "compatible",
  );
  const [baseUrl, setBaseUrl] = useState(
    initial?.baseUrl ?? settings?.baseUrl ?? PROVIDERS[0]!.baseUrl,
  );
  const [model, setModel] = useState(
    initial?.model ?? settings?.defaultModel ?? PROVIDERS[0]!.model,
  );
  const [subscriptionModel, setSubscriptionModel] = useState(
    initial?.subscriptionModel ?? settings?.subscriptionModel ?? "",
  );
  const [effort, setEffort] = useState(initial?.effort ?? settings?.reasoningEffort ?? "auto");
  const [apiKey, setApiKey] = useState(initial?.apiKey ?? "");
  const [told] = useState(() =>
    failure === undefined || !initial
      ? { keyError: "", error: "" }
      : explainFailure(failure, {
          name: providerFor(initial.baseUrl)?.name ?? "This endpoint",
          address: initial.baseUrl.trim(),
          savedOnly: false,
        }),
  );
  const [keyError, setKeyError] = useState(told.keyError);
  const keyField = useId();
  const keyProblem = useId();
  const first = useRef<HTMLButtonElement>(null);
  const keyInput = useRef<HTMLInputElement>(null);
  const testButton = useRef<HTMLButtonElement>(null);
  const [subscription, setSubscription] = useState<SubscriptionStatus | null>(null);
  const [pendingCode, setPendingCode] = useState<DeviceCode | null>(null);
  const [claude, setClaude] = useState<ClaudeProgram | null | undefined>(undefined);
  const [claudeSignedIn, setClaudeSignedIn] = useState(false);
  const [pickups, setPickups] = useState<Pickup[]>([]);
  const [skipped, setSkipped] = useState(false);
  const [busy, setBusy] = useState<"signin" | "testing" | "saving" | null>(null);
  const [error, setError] = useState(told.error);
  const [advanced, setAdvanced] = useState(false);
  const blockedHint = useId();
  const attempt = useRef(0);
  const request = useRef<Request>({ kind: "idle" });

  useEffect(() => {
    if (waiting) return;
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
  }, [waiting]);

  useEffect(() => {
    if (waiting) return;
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
  }, [waiting]);

  useEffect(() => {
    if (waiting) onDraft?.({ provider, baseUrl, model, apiKey, subscriptionModel, effort });
  }, [waiting, onDraft, provider, baseUrl, model, apiKey, subscriptionModel, effort]);

  const offerHere = !waiting && capabilities.borrowedSignins && hostPlace() === "thisMac";
  const unconfigured = provider === "compatible" && !settings?.apiKeySet;

  useEffect(() => {
    if (!offerHere || !unconfigured) return;
    let live = true;
    void api.existingSignins().then(
      (found) => {
        if (live) setPickups(found);
      },
      () => {
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
  const touched =
    provider !== "compatible" ||
    Boolean(apiKey.trim()) ||
    advanced ||
    baseUrl !== (settings?.baseUrl ?? PROVIDERS[0]!.baseUrl);
  const offer = offerHere && unconfigured && !skipped && !touched && pickups.length > 0;

  useEffect(() => {
    first.current?.focus({ preventScroll: true });
  }, [offer]);

  useEffect(() => {
    if (busy) return;
    if (keyError) keyInput.current?.focus({ preventScroll: true });
    else if (error) testButton.current?.focus({ preventScroll: true });
  }, [busy, keyError, error]);

  const claudeBlurb = !capabilities.claudeProvider
    ? "This host does not offer the claude program, so a plan cannot pay for turns here."
    : claude?.installed === false
      ? `Runs the claude program, which this host does not have. Install it there with ${claude.install}, sign in, then come back.`
      : "Use a Claude plan through the claude program on your Guaca host. No API key needed.";

  const blockedReason = waiting
    ? provider === "compatible" && needsKey
      ? `Paste a key for ${preset?.name ?? "this endpoint"}, then continue.`
      : ""
    : !settings
      ? "Waiting for this host to answer with its settings."
      : provider === "compatible" && needsKey
        ? `Paste a key for ${preset?.name ?? "this endpoint"}, then test it.`
        : provider === "claude" && !claudeReady
          ? "This host cannot run the claude program. Choose another card."
          : provider === "claude" && !claudeSignedIn
            ? "Sign in to Claude, then test it."
            : "";

  const choose = (next: Provider) => {
    if (next !== "claude") setClaudeSignedIn(false);
    setProvider(next);
    setError("");
    setKeyError("");
  };

  const changeEndpoint = (value: string) => {
    setBaseUrl(value);
    setApiKey("");
    setError("");
    setKeyError("");
  };

  const abandon = () => {
    attempt.current += 1;
    request.current = transitionRequest(request.current, "finish");
    setPendingCode(null);
    setBusy(null);
    setError("");
  };

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

  const accept = async (pickup: Pickup) => {
    if (request.current.kind === "running" || !settings) return;
    request.current = transitionRequest(request.current, "start");
    const current = ++attempt.current;
    const live = () => attempt.current === current;
    setError("");
    setKeyError("");
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
        request.current = transitionRequest(request.current, "finish");
        setPendingCode(null);
        setBusy(null);
      }
    }
  };

  const connect = async () => {
    if (request.current.kind === "running") return;
    if (provider === "compatible" && (needsKey || !baseUrl.trim() || !model.trim())) {
      setError("Enter an endpoint, a model, and the key for that provider, then try again.");
      return;
    }
    if (waiting) return onConnected();
    if (!settings) return;
    if (provider === "claude" && (!claudeReady || !claudeSignedIn)) return;
    request.current = transitionRequest(request.current, "start");
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
      if (!live()) return;
      const said = explainFailure(failure, {
        name: preset?.name ?? "This endpoint",
        address: baseUrl.trim(),
        savedOnly: savedKey && !apiKey.trim(),
      });
      if (said.keyError) setKeyError(said.keyError);
      else setError(said.error);
    } finally {
      if (live()) {
        request.current = transitionRequest(request.current, "finish");
        setPendingCode(null);
        setBusy(null);
      }
    }
  };

  const saying = error ? (
    <p className="onboarding-notice" role="alert">
      {error}
    </p>
  ) : busy && !pendingCode ? (
    <p className="onboarding-notice" role="status">
      {busy === "signin"
        ? "Starting sign-in…"
        : busy === "testing"
          ? "Testing the connection…"
          : "Saving your provider…"}
    </p>
  ) : null;

  const code = pendingCode && (
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
        Only enter this code if you started this sign-in here. If you cancel, do not finish it in
        the browser.
      </p>
    </div>
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
          {pickups.map((pickup, index) => (
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
                ref={index === 0 ? first : undefined}
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
        <div className="onboarding-actions">
          {code}
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
          {saying}
        </div>
      </section>
    );

  return (
    <section className="onboarding-provider" aria-label="Connect a provider">
      <h2>Choose how your agents are paid for</h2>
      <p>
        {waiting
          ? "If you already pay for ChatGPT or Claude, sign in with that plan and skip the key. Sign-in runs on your host, so it starts once your host is ready; a key can go in now."
          : "If you already pay for ChatGPT or Claude, sign in with that plan and skip the key."}
      </p>
      <div className="onboarding-options onboarding-options--three">
        <button
          type="button"
          className="onboarding-choice"
          ref={provider === "chatgpt" ? first : undefined}
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
          ref={provider === "claude" ? first : undefined}
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
          ref={provider === "compatible" ? first : undefined}
          aria-pressed={provider === "compatible"}
          disabled={Boolean(busy)}
          onClick={() => choose("compatible")}
        >
          <strong>API key</strong>
          <span>Use OpenRouter or OpenAI, with usage billed to that account.</span>
        </button>
      </div>

      {provider === "compatible" && (
        <div className="onboarding-endpoint">
          <label className="field">
            <span className="field__label">API provider</span>
            <select
              className="input"
              value={preset?.id === "openrouter" || preset?.id === "openai" ? preset.id : "custom"}
              disabled={Boolean(busy)}
              onChange={(event) => {
                if (event.target.value === "custom") {
                  changeEndpoint("");
                  setModel("");
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
          <div className="field">
            <label className="field__label" htmlFor={keyField}>
              API key
            </label>
            <input
              ref={keyInput}
              id={keyField}
              className="input"
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={apiKey}
              disabled={Boolean(busy)}
              aria-invalid={keyError ? true : undefined}
              aria-describedby={keyError ? keyProblem : undefined}
              data-hold="key"
              placeholder={savedKey ? "Saved key will be used" : "Paste your provider's API key"}
              onChange={(event) => {
                setApiKey(event.target.value);
                setKeyError("");
              }}
            />
            {keyError && (
              <p className="field__error" id={keyProblem} role="alert">
                {keyError}
              </p>
            )}
          </div>
          {preset?.keyPage && !savedKey && (
            <div className="onboarding-keyhelp">
              <p>
                No {preset.name} key yet?{" "}
                <a
                  href={preset.keyPage}
                  target="_blank"
                  rel="noreferrer"
                  className="access__docs"
                  onClick={(event) => {
                    event.preventDefault();
                    const current = attempt.current;
                    void openExternal(preset.keyPage!).catch(() => {
                      if (attempt.current === current)
                        setError(
                          `The browser could not open. Go to ${preset.keyPage} in your browser.`,
                        );
                    });
                  }}
                >
                  Get a key
                </a>{" "}
                from {new URL(preset.keyPage).hostname} in your browser, then paste it here.
              </p>
              {preset.keySteps && (
                <ol>
                  {preset.keySteps.map((line) => (
                    <li key={line}>{line}</li>
                  ))}
                </ol>
              )}
            </div>
          )}
          <label className="field">
            <span className="field__label">Inference endpoint</span>
            <input
              className="input"
              value={baseUrl}
              disabled={Boolean(busy)}
              onChange={(event) => changeEndpoint(event.target.value)}
            />
          </label>
          <label className="field">
            <span className="field__label">Default model</span>
            <input
              className="input"
              value={model}
              placeholder={preset?.model || "A model your endpoint serves"}
              disabled={Boolean(busy)}
              onChange={(event) => setModel(event.target.value)}
            />
          </label>
          <p className="field__hint">
            {waiting
              ? "Your host is still starting, so nothing is tested yet. Guaca keeps the key in this window's memory only, then tests it and stores it on your host the moment the host is ready."
              : savedKey
                ? "Leave the key blank to keep the one saved for this endpoint."
                : preset
                  ? `Your ${preset.name} key is stored only on your Guaca host.`
                  : "Any OpenAI-compatible endpoint works, and one on your own machine may not need a key."}
            {!capabilities.loopbackEndpoints &&
              " This host cannot reach model endpoints on your own computer."}
          </p>
        </div>
      )}

      {provider === "chatgpt" && (
        <div className="onboarding-notice">
          <p>
            Turns use your ChatGPT plan's Codex quota, not API credits. A group can reach that quota
            faster than one person.
          </p>
          {waiting && (
            <p>
              ChatGPT sign-in runs on your Guaca host, which is still starting. Continue now, and
              Guaca asks you to sign in the moment your host is ready.
            </p>
          )}
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

      {provider === "claude" && waiting && (
        <div className="onboarding-notice">
          Claude sign-in runs on your Guaca host, which is still starting. Continue now, and Guaca
          asks you to sign in the moment your host is ready.
        </div>
      )}

      {provider === "claude" && claudeReady && !waiting && (
        <div className="onboarding-notice">
          Claude uses your plan's own model settings. Testing sends one small request.
          <ClaudeSignin onReady={setClaudeSignedIn} />
        </div>
      )}

      {provider === "chatgpt" && !waiting && (
        <details
          className="onboarding-advanced"
          open={advanced}
          onToggle={(event) => setAdvanced(event.currentTarget.open)}
        >
          <summary>Advanced options</summary>
          <fieldset disabled={Boolean(busy)}>
            {advanced && subscription?.signedIn && (
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
      )}

      <div className="onboarding-actions">
        {code}
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
            ref={testButton}
            type="button"
            className="btn btn--primary"
            aria-describedby={blockedReason && !busy ? blockedHint : undefined}
            disabled={
              Boolean(busy) ||
              Boolean(provider === "compatible" && needsKey) ||
              (!waiting &&
                (!settings || (provider === "claude" && (!claudeReady || !claudeSignedIn))))
            }
            onClick={() => void connect()}
          >
            {waiting
              ? "Continue"
              : provider === "chatgpt" && (!subscription?.signedIn || ineligible)
                ? "Sign in with ChatGPT"
                : "Test and continue"}
          </button>
        )}
        {saying ??
          (blockedReason ? (
            <span id={blockedHint}>{blockedReason}</span>
          ) : waiting ? (
            <span>
              {waitsForHost(provider)
                ? "Sign-in waits for your host. Everything else can go ahead."
                : "Nothing leaves this window until your host is ready."}
            </span>
          ) : (
            provider === "compatible" && (
              <span>Testing makes a small model request that may be billed.</span>
            )
          ))}
      </div>
    </section>
  );
}
