import { useEffect, useMemo, useState } from "react";

import { api } from "../lib/ipc";
import { roleFor } from "../lib/roles";
import type { RankedModel } from "../lib/types";

interface Props {
  name: string;
  skills: string[];
  instructions: string;
  model: string;
  active: boolean;
  onChoose: (model: string) => void;
}

const OFFERED = 3;

export function ModelSuggestions({ name, skills, instructions, model, active, onChoose }: Props) {
  const [ranked, setRanked] = useState<RankedModel[]>([]);
  const [unreachable, setUnreachable] = useState(false);

  const role = useMemo(() => roleFor({ name, skills, instructions }), [name, skills, instructions]);

  const useCase = active ? role?.id : undefined;
  useEffect(() => {
    if (!useCase) {
      setRanked([]);
      setUnreachable(false);
      return;
    }
    let canceled = false;
    void api
      .rankedModels(useCase)
      .then((models) => {
        if (canceled) return;
        setRanked(models);
        setUnreachable(false);
      })
      .catch(() => {
        if (canceled) return;
        setRanked([]);
        setUnreachable(true);
      });
    return () => {
      canceled = true;
    };
  }, [useCase]);

  if (!role || !active) return null;

  const offers = ranked.filter((entry) => entry.id !== model.trim()).slice(0, OFFERED);
  if (offers.length === 0 && !unreachable) return null;

  return (
    <div className="field">
      <span className="field__hint" style={{ display: "block", marginBottom: "0.35rem" }}>
        {unreachable ? (
          <>
            This reads as {role.label} work, but OpenRouter did not answer when asked which models
            get it. Type a slug as usual.
          </>
        ) : (
          <>
            This reads as {role.label} work. These are the models OpenRouter ranks highest for it
            today, most capable first. Pressing one fills the field in.
          </>
        )}
      </span>

      {offers.map((offer) => (
        <button
          key={offer.id}
          type="button"
          className="preset"
          aria-label={`Use ${offer.name} for ${role.label} work`}
          onClick={() => onChoose(offer.id)}
        >
          <span className="preset__text">
            <span className="preset__name">{offer.name}</span>
            <span className="preset__url">{offer.id}</span>
          </span>
          <span className="preset__state" title={exactly(offer)}>
            {roughly(offer)}
          </span>
        </button>
      ))}
    </div>
  );
}

function roughly(model: RankedModel): string {
  const { promptPerMillion: prompt, completionPerMillion: completion } = model;
  if (prompt === null || completion === null) return "no price";
  if (prompt === 0 && completion === 0) return "free";
  return `${dollars(prompt)} / ${dollars(completion)}`;
}

function dollars(value: number): string {
  if (value === 0) return "$0";
  if (value < 0.01) return "<$0.01";
  return `$${value.toFixed(2)}`;
}

function exactly(model: RankedModel): string {
  const { promptPerMillion: prompt, completionPerMillion: completion } = model;
  if (prompt === null || completion === null) {
    return `${model.name} quotes no price on OpenRouter`;
  }
  const context = `${Math.round(model.contextLength / 1000)}K context`;
  return `$${trim(prompt)} per million prompt tokens, $${trim(completion)} per million completion tokens · ${context}`;
}

function trim(value: number): string {
  if (value === 0) return "0";
  return Number(value.toPrecision(3)).toString();
}
