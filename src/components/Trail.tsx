import { useId, useState } from "react";

import { type DiffLine, diffSummary } from "../lib/diff";
import {
  foldTrail,
  hasDetail,
  type Step,
  saysMore,
  stepDiff,
  stepReason,
  type TrailGroup,
  tellsMore,
} from "../lib/trail";
import { FootnoteRow, why } from "./WireRow";

/**
 * The same work, as one line of a page's footnotes.
 *
 * A turn that reads a file, searches and runs a command is three things the
 * operator asked for by asking for the turn, and set as three chips they were
 * three blocks between two paragraphs. As a note they are one line under the
 * paragraph, in the size a note is set in, and the file names are what is read
 * at — which is what a footnote is for.
 *
 * What folds is unchanged: `foldTrail` decides the groups and `hasDetail` says
 * whether one opens, exactly as the chips do. This is a second drawing of the
 * same rule, not a second rule.
 */
export function TrailFootnote({
  steps,
  number,
  at,
}: {
  steps: Step[];
  number: number;
  /** When the turn that made them was written, where the note stands alone. */
  at?: number;
}) {
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
  const failed = groups.some((group) => group.failed);

  return (
    <FootnoteRow number={number} at={at} failed={failed}>
      <span className="footnote__line">
        {groups.map((group, at) => (
          <span className="footnote__item" key={group.key}>
            {at > 0 && (
              <span className="footnote__dot" aria-hidden="true">
                ·
              </span>
            )}
            <FootnoteGroup
              group={group}
              id={`${region}-${group.key}`}
              open={open.has(group.key)}
              onToggle={() => toggle(group.key)}
            />
          </span>
        ))}
      </span>

      {groups
        .filter((group) => open.has(group.key))
        .map((group) => (
          <ul className="footnote__steps" id={`${region}-${group.key}`} key={group.key}>
            {group.steps.map((step) => (
              <StepRow key={step.key} step={step} />
            ))}
          </ul>
        ))}
    </FootnoteRow>
  );
}

/**
 * One kind of work in a note: its label, what it was pointed at, and whether
 * there is more behind it.
 *
 * A single call whose whole content is the sentence on the line gets no
 * control, for the reason a chip with nothing behind it does not: a button
 * that does nothing is one the operator stops trusting the rest of them for.
 */
function FootnoteGroup({
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
  // Only where nothing opens behind the line. A call with detail draws its
  // target in the panel underneath, and printing it here as well said the same
  // filename twice a line apart.
  const target =
    only &&
    !hasDetail(group) &&
    only.target &&
    only.target.length <= 48 &&
    !only.target.includes("\n")
      ? only.target
      : null;
  // A failure that has detail behind it still says why on the line: a refusal
  // is the one part of a note somebody may have to act on, and a footnote that
  // hid the reason behind a click would be hiding the only thing worth reading.
  const failed = only ? stepReason(only) : null;
  const face = (
    <>
      <span className="footnote__label">{group.label}</span>
      {target && <span className="footnote__target">{target}</span>}
      {failed && <span className="footnote__gist">{why(failed)}</span>}
      {group.steps.length > 1 && <span className="footnote__count">×{group.steps.length}</span>}
      {group.spent.map((credential) => (
        <span className="trail__spent" key={credential}>
          {credential}
        </span>
      ))}
    </>
  );

  if (!hasDetail(group) && (!only || !stepReason(only))) {
    return <span className="footnote__call">{face}</span>;
  }

  return (
    <button
      type="button"
      className="footnote__call footnote__call--open"
      aria-expanded={open}
      aria-controls={open ? id : undefined}
      onClick={onToggle}
    >
      {face}
    </button>
  );
}

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
  // A refusal on the chip is a clipped copy of what the chip opens onto, which
  // is the whole of it. Worth having shut, where it is the only thing saying
  // what went wrong; a repeat of the paragraph directly under it once it is
  // open.
  const previewed = only && saysMore(only) && !(open && stepReason(only));
  const face = (
    <>
      <span className="trail__label">{group.label}</span>
      {/* Only where it belongs to one call and adds something the label has
          not already said. Several calls have several answers, and a chip
          quoting whichever happened to be first is a chip that is wrong about
          the rest. The `title` is the part that was cut: a chip is one line,
          and what a routine was set for or a page said does not fit in one. */}
      {previewed && (
        <span className="trail__said" title={only.said}>
          {only.said}
        </span>
      )}
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
 *
 * A call that overwrote something is drawn against what it overwrote instead,
 * however short it is: the whole reason to open a memory rewrite is to find out
 * what the agent changed its mind about, and the content alone never says.
 *
 * A call that went wrong and has nothing else to show is drawn as its reason,
 * for the plainer reason that the reason is what happened. Where that reason is
 * a translation of what the runtime wrote for the model, it is prose: the
 * runtime's own sentence said "`repeat` or `every_secs`", and a parameter name
 * in a monospace block reads as a traceback rather than as an answer. The
 * machine's version stays one click away, which is where somebody diagnosing a
 * refusal wants it and where nobody else has to read it.
 */
function StepRow({ step }: { step: Step }) {
  const changed = stepDiff(step);
  const reason = stepReason(step);
  // A url, a pair of coordinates or an element number is a few characters, and
  // a few characters in a block of their own is a gray rectangle drawn around
  // nothing. A command and a rewritten memory are the reason the block exists.
  const inline =
    !changed && step.target !== null && step.target.length <= 48 && !step.target.includes("\n");

  return (
    <li className="trail__step" data-failed={step.failed || undefined}>
      <div className="trail__step-head">
        <span className="trail__step-title">{step.title}</span>
        {inline && <span className="trail__inline">{step.target}</span>}
        {/* In the place the runtime's own summary would have gone, and for a
            memory rewrite that summary is a character count printed directly
            above the characters. How much of the page moved is the thing the
            operator came here to find out. */}
        {changed && <span className="trail__said">{diffSummary(changed)}</span>}
        {/* Not twice. A refusal drawn below is not also a clipped copy of
            itself on the line above the block holding it. The `title` is the
            part a narrow row cut, for the reason the chip has one. */}
        {step.said && tellsMore(step) && !reason && (
          <span className="trail__said" title={step.said}>
            {step.said}
          </span>
        )}
        {step.spent.map((credential) => (
          <span className="trail__spent" key={credential}>
            {credential}
          </span>
        ))}
      </div>
      {changed && <DiffBlock lines={changed} />}
      {reason &&
        (step.told === null ? (
          <pre className="trail__target">{reason}</pre>
        ) : (
          <p className="trail__why">{reason}</p>
        ))}
      {!changed && step.target && !inline && <pre className="trail__target">{step.target}</pre>}
      {step.told !== null && (
        <details className="trail__told">
          <summary>What the agent was told</summary>
          <pre className="trail__target">{step.told}</pre>
        </details>
      )}
    </li>
  );
}

/**
 * Two versions of a page, drawn as the lines between them.
 *
 * Every line of both is here, unchanged ones included, so the panel answers
 * what the agent now believes as well as what it just decided. A page is short
 * enough to show whole; folding the parts that held still is a saving worth
 * making in a repository and not in a paragraph.
 *
 * The marker is a character rather than only a color, because red and green
 * either side of a line are the one distinction a reader may not have, and
 * because it is what makes a copied diff still read as one.
 */
export function DiffBlock({ lines }: { lines: DiffLine[] }) {
  const mark = { same: " ", added: "+", removed: "-" };
  return (
    <pre className="diff">
      {lines.map((line, at) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: two identical lines in one document are ordinary, and the list is rebuilt whole rather than reordered
        <span className="diff__line" data-kind={line.kind} key={at}>
          <span className="diff__mark">{mark[line.kind]}</span>
          <span className="diff__text">{line.text}</span>
        </span>
      ))}
    </pre>
  );
}
