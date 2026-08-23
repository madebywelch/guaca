/**
 * Three models for the agent being written, under the field that holds one.
 *
 * The model field is a text box on purpose: any slug the endpoint accepts, and
 * the endpoint is the operator's to choose. That is right and it is also a blank
 * box, and a blank box is no help at all to somebody who has just decided they
 * want an agent that reads contracts. This is the help, and it stays a text box.
 *
 * Nothing here is a setting. It reads what the operator has already written into
 * the dialog, asks OpenRouter which models get that kind of work, and offers to
 * fill the field in. Pressing one is the same edit as typing, and the dialog is
 * not saved by it.
 *
 * ## When it says nothing
 *
 * Three ways, and all three are the common case rather than a failure:
 *
 * - The agent is not any of OpenRouter's twelve use cases. Most are not.
 * - The agent's turns are not paid for by OpenRouter, so a slug ranked there
 *   would be a model the endpoint refuses by name.
 * - OpenRouter did not answer. Then it says so, once, quietly: this sits beside
 *   a field that works perfectly well without it.
 */

import { useEffect, useMemo, useState } from "react";

import { api } from "../lib/ipc";
import { roleFor } from "../lib/roles";
import type { RankedModel } from "../lib/types";

interface Props {
  /** The draft as it stands, not the saved card: a suggestion should follow
   *  what is being typed rather than what was typed last time. */
  name: string;
  skills: string[];
  instructions: string;
  /** What is in the model field now, so it is not offered back. */
  model: string;
  /** Whether OpenRouter is what this agent's turns are paid through. */
  active: boolean;
  onChoose: (model: string) => void;
}

/** As many as fit under a field without becoming the field. */
const OFFERED = 3;

export function ModelSuggestions({ name, skills, instructions, model, active, onChoose }: Props) {
  const [ranked, setRanked] = useState<RankedModel[]>([]);
  const [unreachable, setUnreachable] = useState(false);

  // Every keystroke in three fields runs this, so it is a keyword scan and not
  // a model call: `lib/roles.ts` has the argument.
  const role = useMemo(() => roleFor({ name, skills, instructions }), [name, skills, instructions]);

  // Keyed on the use case rather than on the evidence. Typing "Leg" through to
  // "Legal Counsel" derives the same role five times over, and a request per
  // keystroke would be five round trips for one answer. The backend caches for
  // hours on top of that, so re-opening the dialog costs nothing at all.
  const useCase = active ? role?.id : undefined;
  useEffect(() => {
    if (!useCase) {
      setRanked([]);
      setUnreachable(false);
      return;
    }
    let cancelled = false;
    void api
      .rankedModels(useCase)
      .then((models) => {
        if (cancelled) return;
        setRanked(models);
        setUnreachable(false);
      })
      .catch(() => {
        if (cancelled) return;
        setRanked([]);
        setUnreachable(true);
      });
    return () => {
      cancelled = true;
    };
  }, [useCase]);

  if (!role || !active) return null;

  // The model already in the box is not a suggestion. Offering it back gives
  // the operator a button that does nothing and reads as one that is broken.
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
          {/* The ranking is by capability and ignores price, and the most
              capable model in a pool is regularly the dearest thing in it. A
              row that names a model and hides what it costs is a one-click way
              to make every turn of this agent forty times dearer. */}
          <span className="preset__state" title={exactly(offer)}>
            {roughly(offer)}
          </span>
        </button>
      ))}
    </div>
  );
}

/**
 * Prompt and completion price, at the width of the slot it goes in.
 *
 * Rounded hard on purpose: this is here to say cheap or dear at a glance, and a
 * model that costs $0.0000488 per million tokens deserves fewer characters than
 * one that costs $15. The exact figures are on the title, which is where
 * somebody comparing two models rather than glancing at one will look.
 */
function roughly(model: RankedModel): string {
  const { promptPerMillion: prompt, completionPerMillion: completion } = model;
  if (prompt === null || completion === null) return "no price";
  if (prompt === 0 && completion === 0) return "free";
  return `${dollars(prompt)} / ${dollars(completion)}`;
}

function dollars(value: number): string {
  if (value === 0) return "$0";
  // Everything under a cent is the same answer to the question being asked, and
  // spelling out six leading zeros in a 0.6rem monospace slot answers a
  // different one.
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

/** Three significant figures, without the exponent or the trailing zeros that
 *  `toPrecision` leaves behind on a round number. */
function trim(value: number): string {
  if (value === 0) return "0";
  return Number(value.toPrecision(3)).toString();
}
