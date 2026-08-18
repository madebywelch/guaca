import { useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import {
  type ComputerProvider,
  type ComputerProviderStatus,
  errorMessage,
  type GuardLimits,
  type Provider,
  type ProviderReadiness,
} from "../lib/types";

interface Props {
  onClose: () => void;
}

/** What each choice is called, and what it costs to read in one line. */
const PROVIDERS: { value: ComputerProvider; label: string }[] = [
  { value: "automatic", label: "Automatic (recommended)" },
  { value: "appleContainer", label: "Apple Container — local" },
  { value: "e2b", label: "E2B — hosted" },
];

const PROVIDER_NAMES: Record<Provider, string> = {
  appleContainer: "Apple Container",
  e2b: "E2B",
};

/** The state as a word. The detail says what to do; this says where it stands. */
const READINESS: Record<ProviderReadiness, string> = {
  ready: "ready",
  notInstalled: "not installed",
  notRunning: "not running",
  unsupported: "unsupported",
  error: "error",
};

/** Whether a provider could make a machine now, or start something and then. */
function usable(status: ComputerProviderStatus): boolean {
  return status.state === "ready" || status.canStart;
}

/**
 * Whether the next computer would run on this Mac.
 *
 * Named directly, or picked by `automatic` because the local provider is the
 * first thing it tries and it is ready. An operator who leaves the setting
 * alone on a Mac with Apple Container installed is choosing local, and should
 * read the same disclosure as one who said so.
 */
function runsLocally(choice: ComputerProvider, statuses: ComputerProviderStatus[]): boolean {
  if (choice === "appleContainer") return true;
  if (choice !== "automatic") return false;
  return statuses.some((status) => status.provider === "appleContainer" && usable(status));
}

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

export function SettingsDialog({ onClose }: Props) {
  const settings = useStore((s) => s.settings);
  const setSettings = useStore((s) => s.setSettings);
  const statuses = useStore((s) => s.computerStatuses);
  const refreshStatuses = useStore((s) => s.refreshComputerStatuses);

  const [operatorName, setOperatorName] = useState(settings?.operatorName ?? "");
  const [baseUrl, setBaseUrl] = useState(settings?.baseUrl ?? "");
  const [model, setModel] = useState(settings?.defaultModel ?? "");
  const [apiKey, setApiKey] = useState("");
  const [e2bKey, setE2bKey] = useState("");
  const [provider, setProvider] = useState<ComputerProvider>(
    settings?.computerProvider ?? "automatic",
  );
  const [idleMinutes, setIdleMinutes] = useState("");
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

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Asked when the dialog opens rather than only at startup: a runtime the
  // operator installed or started since then answers differently, and this is
  // the screen they came to to find that out.
  useEffect(() => {
    void refreshStatuses();
  }, [refreshStatuses]);

  const save = async () => {
    setBusy(true);
    setStatus(null);
    try {
      const next = await api.updateSettings({
        operatorName,
        baseUrl,
        defaultModel: model,
        limits,
        computerProvider: provider,
        // Omitted when blank, so saving without retyping keeps the stored key.
        ...(apiKey.trim() ? { apiKey: apiKey.trim() } : {}),
        ...(e2bKey.trim() ? { e2bApiKey: e2bKey.trim() } : {}),
        ...(idleMinutes.trim() ? { computerIdleMinutes: Number(idleMinutes) } : {}),
      });
      setSettings(next);
      setApiKey("");
      // A key just added is a provider that can suddenly make a machine, and
      // the lines below say the opposite until somebody asks again.
      void refreshStatuses();
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
      const result = await api.testConnection({
        baseUrl,
        defaultModel: model,
        ...(apiKey.trim() ? { apiKey: apiKey.trim() } : {}),
        ...(e2bKey.trim() ? { e2bApiKey: e2bKey.trim() } : {}),
        ...(idleMinutes.trim() ? { computerIdleMinutes: Number(idleMinutes) } : {}),
      });
      setStatus({ tone: "ok", text: result });
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
      <div className="dialog" role="dialog" aria-modal="true" aria-label="Settings">
        <h2 className="dialog__title">Settings</h2>
        <p className="dialog__lede">
          Guaca runs entirely on this machine. The only thing it sends anywhere is what you and your
          agents type, to the endpoint below.
        </p>

        <label className="field">
          <span className="field__label">Your name</span>
          <input
            className="input"
            value={operatorName}
            placeholder="Unnamed"
            onChange={(event) => setOperatorName(event.target.value)}
          />
          <span className="field__hint">
            What every agent calls you. They are told this on every turn, so you never have to
            introduce yourself or ask one to remember it. Leave blank and they say "the operator".
          </span>
        </label>

        <label className="field">
          <span className="field__label">Inference endpoint</span>
          <input
            className="input input--mono"
            value={baseUrl}
            placeholder="https://openrouter.ai/api/v1"
            onChange={(event) => setBaseUrl(event.target.value)}
          />
          <span className="field__hint">
            Any OpenAI-compatible base URL. Point it at a local server to run without a network.
          </span>
        </label>

        <label className="field">
          <span className="field__label">API key</span>
          <input
            className="input input--mono"
            type="password"
            value={apiKey}
            placeholder={settings?.apiKeySet ? `Stored ${settings.apiKeyHint}` : "sk-or-v1-…"}
            autoComplete="off"
            onChange={(event) => setApiKey(event.target.value)}
          />
          <span className="field__hint">
            Stored on this machine in a file only your user account can read. Leave blank to keep
            the current key.
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
          <span className="field__hint">Used for new agents. Each agent can override it.</span>
        </label>

        <hr className="divider" />

        <h3 className="dialog__title" style={{ fontSize: "0.85rem" }}>
          Computers
        </h3>
        <p className="dialog__lede">
          An agent's computer is a Linux machine with a desktop, a browser and a shell, shown in the
          corner of its channel. It can run here on this Mac or in a hosted sandbox.
        </p>

        <label className="field">
          <span className="field__label">Computer provider</span>
          <select
            className="select"
            value={provider}
            onChange={(event) => setProvider(event.target.value as ComputerProvider)}
          >
            {PROVIDERS.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </select>
          <span className="field__hint">
            Who runs computers made from now on. Automatic takes the first one below that is ready.
          </span>
        </label>

        <ul className="providers">
          {statuses.map((status) => (
            <li className="provider" key={status.provider}>
              <span className="provider__name">{PROVIDER_NAMES[status.provider]}</span>
              <span className="provider__state" data-state={status.state}>
                {READINESS[status.state]}
              </span>
              {/* The provider's own words. It knows what is missing on this
                  Mac; this dialog would be guessing. */}
              <span className="provider__detail">{status.detail}</span>
            </li>
          ))}
        </ul>

        {runsLocally(provider, statuses) && (
          <blockquote className="disclosure">
            Local computers run untrusted agent commands on this Mac. They cannot see host files
            unless shared, but they may reach services exposed by this Mac or its local network. Use
            E2B when you need an off-device network boundary.
          </blockquote>
        )}

        {provider !== (settings?.computerProvider ?? "automatic") && (
          <p className="field__hint" style={{ margin: "0 0 0.95rem" }}>
            Existing computers keep their current provider until you destroy them.
          </p>
        )}

        <label className="field">
          <span className="field__label">E2B API key</span>
          <input
            className="input input--mono"
            type="password"
            value={e2bKey}
            placeholder={settings?.e2bKeySet ? `Stored ${settings.e2bKeyHint}` : "e2b_… (optional)"}
            autoComplete="off"
            onChange={(event) => setE2bKey(event.target.value)}
          />
          <span className="field__hint">
            What the hosted provider runs on. Not needed for a local computer.
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
            Idle minutes before a machine sleeps. Sleeping keeps its disk, so a browser stays signed
            in and wakes where it left off. A hosted machine is only billed while it runs; a local
            one only takes this Mac's memory while it runs.
          </span>
        </label>

        <hr className="divider" />

        <h3 className="dialog__title" style={{ fontSize: "0.85rem" }}>
          Limits
        </h3>
        <p className="dialog__lede">
          Agents that message each other do not stop on their own. These bounds decide when a
          conversation ends, and every agent is told why when it hits one.
        </p>

        {LIMITS.map((field) => (
          <label className="field" key={field.key}>
            <span className="field__label">{field.label}</span>
            <input
              className="input input--mono"
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

        {status && (
          <div
            className={
              status.tone === "error"
                ? "banner banner--error"
                : status.tone === "ok"
                  ? "banner banner--ok"
                  : "banner"
            }
            style={{ margin: "0 0 0.9rem" }}
          >
            <span>{status.text}</span>
          </div>
        )}

        <div style={{ display: "flex", gap: "0.5rem" }}>
          <button type="button" className="btn" disabled={busy} onClick={() => void test()}>
            Test connection
          </button>
          <span className="hint" style={{ alignSelf: "center" }}>
            Tests what is on screen. Save to keep it.
          </span>
          <div style={{ marginLeft: "auto", display: "flex", gap: "0.5rem" }}>
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
    </div>
  );
}
