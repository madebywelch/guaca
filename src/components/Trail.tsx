import { useId, useState } from "react";

import {
  foldTrail,
  hasDetail,
  type Step,
  saysMore,
  type TrailGroup,
  tellsMore,
} from "../lib/trail";

/**
 * What an agent did on its own turn, collapsed.
 *
 * The quietest row a channel draws, and the one that used to be the loudest by
 * volume: a browsing turn spends most of its twenty-four rounds in the browser,
 * and every one of those was a line of its own reading `Chef used browse`. Now
 * a turn is chips, one per kind of work, and the calls behind a chip open in
 * place.
 *
 * Nobody is named. There is exactly one agent whose own work this can be, its
 * portrait and name are at the top of the pane, and the row this replaced put
 * that name in front of every line.
 *
 * `lib/trail.ts` decides what folds and what a chip says. This decides only how
 * it is drawn and what a click opens, so the rules can be read and tested
 * without a DOM.
 */
export function TrailRow({ steps }: { steps: Step[] }) {
  const groups = foldTrail(steps);
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set());
  const region = useId();

  if (groups.length === 0) return null;

  const toggle = (key: string) =>
    setOpen((held) => {
      const next = new Set(held);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  return (
    <div className="trail">
      <div className="trail__chips">
        {groups.map((group) => (
          <TrailChip
            key={group.key}
            group={group}
            id={`${region}-${group.key}`}
            open={open.has(group.key)}
            onToggle={() => toggle(group.key)}
          />
        ))}
      </div>

      {groups
        .filter((group) => open.has(group.key))
        .map((group) => (
          <ul className="trail__steps" id={`${region}-${group.key}`} key={group.key}>
            {group.steps.map((step) => (
              <StepRow key={step.key} step={step} />
            ))}
          </ul>
        ))}
    </div>
  );
}

/**
 * One kind of work, and whether there is more of it to read.
 *
 * A chip is only a button where something opens: a directory lookup is one call
 * whose whole content is the sentence already on the chip, and a control that
 * does nothing is one the operator stops trusting the rest of.
 */
function TrailChip({
  group,
  id,
  open,
  onToggle,
}: {
  group: TrailGroup;
  id: string;
  open: boolean;
  onToggle: () => void;
}) {
  const only = group.steps.length === 1 ? group.steps[0]! : null;
  const face = (
    <>
      <span className="trail__label">{group.label}</span>
      {/* Only where it belongs to one call and adds something the label has
          not already said. Several calls have several answers, and a chip
          quoting whichever happened to be first is a chip that is wrong about
          the rest. */}
      {only && saysMore(only) && <span className="trail__said">{only.said}</span>}
      {group.spent.map((credential) => (
        <span className="trail__spent" key={credential}>
          {credential}
        </span>
      ))}
    </>
  );

  if (!hasDetail(group)) {
    return (
      <span className="trail__chip" data-failed={group.failed || undefined}>
        {face}
      </span>
    );
  }

  return (
    <button
      type="button"
      className="trail__chip trail__chip--open"
      data-failed={group.failed || undefined}
      aria-expanded={open}
      aria-controls={open ? id : undefined}
      onClick={onToggle}
    >
      <span className="trail__caret" aria-hidden="true">
        {open ? "▾" : "▸"}
      </span>
      {face}
    </button>
  );
}

/**
 * One call, opened.
 *
 * The command, the URL or the memory that was written, as machine text and
 * exactly as it was: this is a record of what happened, so it is never markdown
 * and never anything a model could format its way out of. Long ones scroll
 * rather than push the transcript sideways.
 */
function StepRow({ step }: { step: Step }) {
  // A url, a pair of coordinates or an element number is a few characters, and
  // a few characters in a block of their own is a grey rectangle drawn around
  // nothing. A command and a rewritten memory are the reason the block exists.
  const inline = step.target !== null && step.target.length <= 48 && !step.target.includes("\n");

  return (
    <li className="trail__step" data-failed={step.failed || undefined}>
      <div className="trail__step-head">
        <span className="trail__step-title">{step.title}</span>
        {inline && <span className="trail__inline">{step.target}</span>}
        {step.said && tellsMore(step) && <span className="trail__said">{step.said}</span>}
        {step.spent.map((credential) => (
          <span className="trail__spent" key={credential}>
            {credential}
          </span>
        ))}
      </div>
      {step.target && !inline && <pre className="trail__target">{step.target}</pre>}
    </li>
  );
}
