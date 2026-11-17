import { type ReactNode, useEffect, useState } from "react";
import { api } from "../lib/ipc";
import { PROVIDERS, type Provider as Preset, providerFor, providerReady } from "../lib/providers";
import { hosted } from "../lib/transport";
import type { SubscriptionModel as Model, ReasoningEffort } from "../lib/types";

interface PresetProps {
  baseUrl: string;
  active: boolean;
  keySet: boolean;
  loopback: boolean;
  onChoose: (preset: Preset) => void;
}

export function ProviderPresets({ baseUrl, active, keySet, loopback, onChoose }: PresetProps) {
  const current = providerFor(baseUrl);
  return (
    <>
      {hosted && (
        <p className="field__hint">
          Addresses are reached from the backend. In a container, localhost is the container; use
          host.docker.internal for a model on the host.
        </p>
      )}
      {PROVIDERS.map((preset) => {
        const chosen = active && current?.id === preset.id;
        const withheld = Boolean(preset.local) && !loopback;
        return (
          <button
            key={preset.id}
            type="button"
            className="preset"
            aria-current={chosen}
            disabled={withheld}
            onClick={() => onChoose(preset)}
          >
            <span className="preset__text">
              <span className="preset__name">{preset.name}</span>
              <span className="preset__url">{preset.baseUrl}</span>
            </span>
            {withheld ? (
              <span className="preset__state" data-ready="false">
                Not from a server
              </span>
            ) : preset.local ? (
              <span className="preset__state" data-ready="true">
                {hosted ? "On the backend" : "On this machine"}
              </span>
            ) : (
              chosen && (
                <span
                  className="preset__state"
                  data-ready={providerReady(preset, keySet) ? "true" : undefined}
                >
                  {providerReady(preset, keySet) ? "Key stored" : "Needs a key"}
                </span>
              )
            )}
          </button>
        );
      })}
    </>
  );
}

interface ModelProps {
  value: string;
  models: string[];
  onChange: (model: string) => void;
  inherit?: string;
  title?: string;
  hint?: ReactNode;
  effort: ReasoningEffort | null;
  onEffortChange: (effort: ReasoningEffort | null) => void;
  inheritedModel?: string;
  inheritedEffort?: ReasoningEffort;
  effortInherit?: string;
}

export function SubscriptionModel({
  value,
  models,
  onChange,
  inherit,
  title,
  hint,
  effort,
  onEffortChange,
  inheritedModel,
  inheritedEffort,
  effortInherit,
}: ModelProps) {
  const [catalog, setCatalog] = useState<Model[] | null>(null);
  const [unavailable, setUnavailable] = useState(false);
  useEffect(() => {
    let canceled = false;
    void api.subscriptionModels().then(
      (loaded) => {
        if (!canceled) setCatalog(loaded);
      },
      () => {
        if (!canceled) setUnavailable(true);
      },
    );
    return () => {
      canceled = true;
    };
  }, []);

  const offered = [
    ...new Set([...(catalog?.map((model) => model.slug) ?? models), value].filter(Boolean)),
  ];
  const selected = catalog?.find((model) => model.slug === (value || inheritedModel));
  const effectiveEffort = effort ?? inheritedEffort ?? "auto";
  const efforts = selected?.reasoningEfforts ?? [];
  const unsupported =
    selected &&
    effectiveEffort !== "auto" &&
    !efforts.some((entry) => entry.effort === effectiveEffort);
  return (
    <>
      <label className="field" style={{ marginTop: "1.1rem" }}>
        <span className="field__label" title={title}>
          Model
        </span>
        <select
          className="input input--mono"
          value={value}
          onChange={(event) => onChange(event.target.value)}
        >
          {inherit !== undefined && <option value="">{inherit}</option>}
          {offered.map((slug) => (
            <option key={slug} value={slug}>
              {slug}
            </option>
          ))}
        </select>
        {hint && <span className="field__hint">{hint}</span>}
        {catalog && value && !catalog.some((model) => model.slug === value) && (
          <span className="field__hint" role="status" style={{ display: "block" }}>
            {value} is not in your account's current ChatGPT model list. Choose a listed model
            {inherit !== undefined ? " or use the default above" : ""} if it no longer works.
          </span>
        )}
        {unavailable && (
          <span className="field__hint" role="status" style={{ display: "block" }}>
            Could not load current ChatGPT models. Showing saved and default choices. Reopen this
            panel to retry.
          </span>
        )}
      </label>
      <label className="field">
        <span className="field__label">Reasoning effort</span>
        <select
          className="input"
          value={effort ?? ""}
          onChange={(event) =>
            onEffortChange((event.target.value || null) as ReasoningEffort | null)
          }
        >
          {effortInherit && (
            <option value="">
              {effortInherit} ·{" "}
              {inheritedEffort === "auto" || !inheritedEffort ? "Model default" : inheritedEffort}
            </option>
          )}
          <option value="auto">
            Model default
            {selected?.defaultReasoningEffort ? ` · ${selected.defaultReasoningEffort}` : ""}
          </option>
          {efforts.map((entry) => (
            <option key={entry.effort} value={entry.effort}>
              {entry.effort}
            </option>
          ))}
          {effort && effort !== "auto" && !efforts.some((entry) => entry.effort === effort) && (
            <option value={effort}>{effort} (saved)</option>
          )}
        </select>
        <span className="field__hint">
          {efforts.find((entry) => entry.effort === effectiveEffort)?.description ??
            "Choose how much reasoning this model uses. Higher effort can take longer and use more of your quota."}
        </span>
        {unsupported && (
          <span className="field__hint" role="status">
            {effectiveEffort} is not offered for {selected.slug}. Choose an available effort or
            Model default before starting a turn.
          </span>
        )}
        {!selected && (
          <span className="field__hint">
            Effort choices load with the model catalog. Saved settings are kept.
          </span>
        )}
      </label>
    </>
  );
}
