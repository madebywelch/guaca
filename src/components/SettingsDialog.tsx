import { useEffect, useState } from "react";

import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { errorMessage, type GuardLimits } from "../lib/types";

interface Props {
  onClose: () => void;
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

  const [baseUrl, setBaseUrl] = useState(settings?.baseUrl ?? "");
  const [model, setModel] = useState(settings?.defaultModel ?? "");
  const [apiKey, setApiKey] = useState("");
  const [limits, setLimits] = useState<GuardLimits>(
    settings?.limits ?? { maxHops: 8, maxStepsPerRun: 60, maxFanoutPerCall: 8, maxSendsPerPair: 6 },
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

  const save = async () => {
    setBusy(true);
    setStatus(null);
    try {
      const next = await api.updateSettings({
        baseUrl,
        defaultModel: model,
        limits,
        // Omitted when blank, so saving without retyping keeps the stored key.
        ...(apiKey.trim() ? { apiKey: apiKey.trim() } : {}),
      });
      setSettings(next);
      setApiKey("");
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
          Guac runs entirely on this machine. The only thing it sends anywhere is what you and your
          agents type, to the endpoint below.
        </p>

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
