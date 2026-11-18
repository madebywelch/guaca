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
import { Icon } from "./Icon";
import { FootnoteRow, why } from "./WireRow";

export function TrailFootnote({
  steps,
  number,
  at,
}: {
  steps: Step[];
  number: number;
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
        {groups.map((group) => (
          <span className="footnote__item" key={group.key}>
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
  const target =
    only &&
    !hasDetail(group) &&
    only.target &&
    only.target.length <= 48 &&
    !only.target.includes("\n")
      ? only.target
      : null;
  const failed = only ? stepReason(only) : null;
  const tools = [...new Set(group.steps.map((step) => step.tool))].join(", ");
  const face = (
    <>
      <span className="footnote__label" title={tools}>
        {group.label}
      </span>
      {target && <span className="footnote__target">{target}</span>}
      {failed && (
        <>
          {": "}
          <span className="footnote__gist">{why(failed)}</span>
        </>
      )}
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
  const previewed = only && saysMore(only) && !(open && stepReason(only));
  const face = (
    <>
      <span className="trail__label">{group.label}</span>
      {previewed && (
        <>
          {only.failed && ": "}
          <span className="trail__said" title={only.said}>
            {only.said}
          </span>
        </>
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
        <Icon name={open ? "chevron-down" : "chevron-right"} size={16} />
      </span>
      {face}
    </button>
  );
}

function StepRow({ step }: { step: Step }) {
  const changed = stepDiff(step);
  const reason = stepReason(step);
  const inline =
    !changed && step.target !== null && step.target.length <= 48 && !step.target.includes("\n");

  return (
    <li className="trail__step" data-failed={step.failed || undefined}>
      <div className="trail__step-head">
        <span className="trail__step-title" title={step.tool}>
          {step.title}
        </span>
        {inline && <span className="trail__inline">{step.target}</span>}
        {changed && <span className="trail__said">{diffSummary(changed)}</span>}
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
