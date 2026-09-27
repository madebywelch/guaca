import { Fragment, memo, useLayoutEffect } from "react";
import { useStore } from "../lib/store";
import { elapsed, useNow } from "../lib/time";
import { callInFlight, type LiveCall, stepDiff, trailStep } from "../lib/trail";
import type { AgentId } from "../lib/types";
import { DiffBlock } from "./Trail";

/** Subscribe only while visible; token updates never redraw settled messages. */
export function TurnWork({ agent, follow }: { agent: AgentId; follow: () => void }) {
  const enabled = useStore((s) => s.prefs.showReasoning);
  return enabled ? <Working agent={agent} follow={follow} /> : null;
}

function Working({ agent, follow }: { agent: AgentId; follow: () => void }) {
  const thought = useStore((s) => s.reasoning[agent] ?? "");
  const calls = useStore((s) => s.trail[agent]);
  useLayoutEffect(follow, [follow, thought, calls]);
  if (!thought && !calls?.length) return null;

  let position = 0;
  return (
    <section className="turn-work" aria-label="Reasoning and tool calls" aria-live="off">
      {calls?.map((call, index) => {
        const end = call.reasoningOffset ?? position;
        const text = thought.slice(position, end);
        position = end;
        return (
          // biome-ignore lint/suspicious/noArrayIndexKey: calls append in arrival order; providers may reuse call IDs across rounds.
          <Fragment key={`${call.callId}-${index}`}>
            {text && <Thinking text={text} />}
            <ToolCall call={call} />
          </Fragment>
        );
      })}
      {thought.slice(position) && <Thinking text={thought.slice(position)} />}
    </section>
  );
}

const Thinking = memo(function Thinking({ text }: { text: string }) {
  return (
    <details className="turn-work__thinking" open>
      <summary>Thinking</summary>
      <pre className="thought__text">{text}</pre>
    </details>
  );
});

/** Stable calls retain their disclosure state and do not redraw for reasoning. */
const ToolCall = memo(function ToolCall({ call }: { call: LiveCall }) {
  const step = call.done ? trailStep(call.done, call.callId) : null;
  const changed = step ? stepDiff(step) : null;
  const status = call.done?.outcome.status;
  const label =
    status === "refused"
      ? "Refused"
      : status === "failed"
        ? "Failed"
        : status === "partial"
          ? "Partially completed"
          : "Done";
  return (
    <details className="turn-work__call" data-failed={step?.failed || undefined}>
      <summary>
        <span className="turn-work__name">{call.name}</span>
        {step?.spent.map((credential) => (
          <span className="trail__spent" key={credential}>
            {credential}
          </span>
        ))}
        <span className="turn-work__status">
          {call.done ? label : <Running since={call.startedAt} />}
        </span>
      </summary>
      <p className="turn-work__description">
        {step ? step.said : callInFlight(call.name, call.arguments)}
      </p>
      <pre className="trail__target">{JSON.stringify(call.arguments, null, 2)}</pre>
      {changed && <DiffBlock lines={changed} />}
    </details>
  );
});

function Running({ since }: { since: number }) {
  const now = useNow(1000);
  return <>Running · {elapsed(now, since)}</>;
}
