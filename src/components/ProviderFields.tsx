/**
 * The two parts of choosing a provider that are drawn twice.
 *
 * Once in Settings, for the app, and once in a group, for one crew. The rest of
 * each pane differs enough that sharing it would be a component with a prop per
 * sentence; these two are here because getting either subtly wrong is expensive
 * and invisible. A misspelled endpoint fails on every turn of every agent with
 * an error from a server rather than from Guaca, and a model list that has
 * drifted from the backend's offers a model the plan refuses by name.
 */

import type { ReactNode } from "react";
import { PROVIDERS, type Provider as Preset, providerFor, providerReady } from "../lib/providers";
import { hosted } from "../lib/transport";

interface PresetProps {
  /** The endpoint in the box, whatever it is. Decides which row reads as
   *  chosen, so it is the live value rather than the stored one. */
  baseUrl: string;
  /** Whether an endpoint is what pays at all. A group following the app, or
   *  anything on a subscription, has no chosen row even with a URL in the box. */
  active: boolean;
  /** Whether the key that belongs to the chosen endpoint is set. */
  keySet: boolean;
  /**
   * Whether the backend permits local model addresses. A hosted workspace
   * resolves these on its own network, which the panel explains explicitly.
   */
  loopback: boolean;
  onChoose: (preset: Preset) => void;
}

/**
 * Endpoints that are known to speak the protocol this app speaks, as rows.
 *
 * A starting point, never a restriction: anything else is typed into the field
 * below, and choosing a row only fills it in.
 */
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
            {/* Only the row that is actually chosen can say anything about the
                key, because there is one key and it belongs to the endpoint in
                the field below. Saying "key stored" against six providers the
                operator has never used, on the strength of a seventh provider's
                key, is the same sentence repeated until it means nothing. Local
                endpoints are the exception: wanting no key is a property of the
                server, not of this setup. */}
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
  /** What the backend says it can run. Held by the backend rather than here so
   *  the two cannot drift. */
  models: string[];
  onChange: (model: string) => void;
  /** Offered as the first row when a blank value means something: a group that
   *  leaves this alone runs on whatever the app is set to. */
  inherit?: string;
  /** What this model is used for, which differs between the app and a group. */
  hint: ReactNode;
}

/**
 * The models a subscription can run, as a list rather than a box.
 *
 * There is nothing to type: the plan decides what is on offer, and a model the
 * plan cannot run is a refusal by name on the next turn. The whole field rather
 * than the control, so the label wraps its own input.
 */
export function SubscriptionModel({ value, models, onChange, inherit, hint }: ModelProps) {
  // Whatever is stored is listed even if it is not one of the known ones, so a
  // model chosen before this list changed is not silently swapped for another.
  const offered = [...new Set([...models, value].filter(Boolean))];
  return (
    <label className="field" style={{ marginTop: "1.1rem" }}>
      <span className="field__label">Model</span>
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
      <span className="field__hint">{hint}</span>
    </label>
  );
}
