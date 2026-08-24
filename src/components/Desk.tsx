import { type FormEvent, useEffect, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { useStore } from "../lib/store";
import type { AgentCard, Approval, Decision } from "../lib/types";

/**
 * Everything waiting on the operator, wherever they are in the app.
 *
 * A parked turn is the one thing in Guaca that stops until a person deals with
 * it, and until now the only complete list of them was in the menu bar, which
 * exists for the time the window is *not* in front of you. With the window open
 * and a dozen crews, a parked turn was a mark on a circle and a card somewhere
 * in a transcript three groups away: noticing it and answering it were six
 * steps apart, and the answer had a ten minute fuse on it.
 *
 * So there are three tiers now, over one queue, and each is a different
 * question. The count on a crew's circle says *where*. This says *what*, and
 * takes the answer. The transcript's card is still the record, and still the
 * place to go when the summary is not enough and the conversation around it is.
 *
 * Four rules keep it from turning into a notification center, and each of them
 * is a thing it refuses to do.
 *
 * **It holds stopped work and nothing else.** A parked turn qualifies by
 * definition. A run that failed does not: nothing is waiting, and the channel
 * is where a failure is understood. Neither does a run that finished, a routine
 * that fired, or a paused agent. Guaca already has a surface for "something
 * happened" and it is a notification; this is for "something has stopped, and
 * you are the reason".
 *
 * **It is usually absent.** No queue, no surface, not even a small empty one.
 * A panel that is always there is furniture within a week, and furniture is not
 * read. Being gone almost all the time is what buys the corner of the screen.
 *
 * **It has no composer and no scrollback.** Every control on it is bounded, and
 * what has been answered is gone from it. The transcript is the record; a
 * second one that could be scrolled back through would eventually disagree with
 * the first, and the operator would have no way to tell which was lying.
 *
 * **It never takes focus.** A request can arrive while the operator is mid
 * sentence in a composer, and a panel that grabbed the caret would lose what
 * they were typing. It announces itself once, politely, and waits.
 */
export function Desk() {
  const pending = useStore((s) => s.pending);
  const agents = useStore((s) => s.agents);
  const select = useStore((s) => s.select);
  const decide = useStore((s) => s.decideApproval);
  const answerQuestion = useStore((s) => s.answerQuestion);
  const [open, setOpen] = useState(true);

  // Opens itself again for a queue that has refilled. Collapsing is about the
  // requests that were on screen at the time, not a standing instruction to
  // keep quiet: a desk that stayed shut after being emptied and refilled would
  // silently hold the one thing that has stopped work.
  const wasEmpty = useRef(true);
  useEffect(() => {
    if (wasEmpty.current && pending.length > 0) setOpen(true);
    wasEmpty.current = pending.length === 0;
  }, [pending.length]);

  // The lowest-priority owner of Escape: a dialog, a menu or a drag all have
  // something more urgent to close, and every one of them stops the event
  // before it reaches the window.
  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open]);

  if (pending.length === 0) return null;

  const count = pending.length;
  const summary = count === 1 ? "1 turn is waiting on you" : `${count} turns are waiting on you`;

  return (
    <section className="desk" aria-label="Waiting on you" data-open={open ? "true" : undefined}>
      <button
        type="button"
        className="desk__head"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <span className="desk__count">{count}</span>
        {/* The live region is the line that is already on screen rather than a
            second copy of it offscreen. Two elements saying the same sentence
            is a screen reader reading it twice, and it is the shape a live
            region usually goes wrong in. Polite, so it waits for a gap: this
            can land while the operator is mid sentence in a composer. */}
        <span className="desk__title" role="status">
          {summary}
        </span>
        <span className="desk__chevron" aria-hidden="true">
          {open ? "▾" : "▴"}
        </span>
      </button>

      {open && (
        <div className="desk__queue">
          {pending.map((request) => (
            <DeskCard
              key={request.id}
              request={request}
              agent={agents.find((a) => a.id === request.agentId)}
              onDecide={(decision) => decide(request.id, decision)}
              onAnswer={(answer) => answerQuestion(request.id, answer)}
              onOpenChannel={() => void select(request.agentId)}
            />
          ))}
        </div>
      )}
    </section>
  );
}

interface CardProps {
  request: Approval;
  /** Who asked. Undefined once it has been deleted. */
  agent: AgentCard | undefined;
  onDecide: (decision: Decision) => Promise<void>;
  onAnswer: (answer: string) => Promise<void>;
  onOpenChannel: () => void;
}

/**
 * One request, answerable where it was noticed.
 *
 * Every value here is what a model asked for, so all of it is drawn as text
 * under a heading Guaca wrote, exactly as it is in the transcript. An agent
 * that could format its own request could draw a button, and a button on this
 * surface is one the operator has been trained to trust.
 */
function DeskCard({ request, agent, onDecide, onAnswer, onOpenChannel }: CardProps) {
  const [answering, setAnswering] = useState(false);
  const asker = agent?.name ?? "A deleted agent";

  // Released whichever way it went. A refused decision leaves the request in
  // the queue, and a card whose buttons stayed disabled after that is a request
  // the operator can see and can no longer answer.
  const answer = (decision: Decision) => {
    setAnswering(true);
    void onDecide(decision).finally(() => setAnswering(false));
  };

  return (
    <article className="desk__card">
      <header className="desk__who">
        <AgentAvatar
          avatar={agent?.avatar ?? "blank"}
          color={agent?.color ?? "#8aa0a6"}
          size="sm"
          seed={agent?.id ?? request.id}
        />
        <div className="desk__whotext">
          <p className="desk__asker">{asker}</p>
          <p className="desk__summary">{request.summary}</p>
        </div>
      </header>

      {request.detail.length > 0 && (
        <dl className="desk__detail">
          {request.detail.map((field) => (
            <div key={`${request.id}:${field.label}`}>
              <dt>{field.label}</dt>
              <dd>{field.value}</dd>
            </div>
          ))}
        </dl>
      )}

      <div className="desk__actions">
        {request.request.kind === "permission" ? (
          <>
            <button
              type="button"
              className="btn btn--primary btn--small"
              disabled={answering}
              onClick={() => answer("allow")}
            >
              Allow
            </button>
            {/* Absent for anything done in the operator's name, for the reason
                the transcript's card gives at length: "always" is scoped to an
                agent and an action, and this action is "act outside the
                workspace". */}
            {request.request.action === "createAgent" && (
              <button
                type="button"
                className="btn btn--small"
                disabled={answering}
                title={`Stop asking when ${asker} does this`}
                onClick={() => answer("alwaysAllow")}
              >
                Always
              </button>
            )}
            <button
              type="button"
              className="btn btn--ghost btn--small"
              disabled={answering}
              onClick={() => answer("deny")}
            >
              Deny
            </button>
          </>
        ) : (
          <Answers
            options={request.request.options}
            disabled={answering}
            onAnswer={(text) => {
              setAnswering(true);
              void onAnswer(text).finally(() => setAnswering(false));
            }}
          />
        )}
        {/* The way out of the summary and into what led to it. A decision that
            needs the conversation around it is exactly the one this surface
            must not pretend it can take. */}
        <button
          type="button"
          className="btn btn--ghost btn--small desk__open"
          onClick={onOpenChannel}
        >
          Open channel
        </button>
      </div>
    </article>
  );
}

/**
 * The answers to a question: either the choices it offered, or one field.
 *
 * The choices are the agent's own words, and they are the only model-authored
 * text anywhere in this app that lands on a button. That is safe here and
 * nowhere else, because nothing answered here authorizes anything: the value
 * goes back to the agent, and whatever it does next passes through every guard
 * it already had. They are drawn as text, never as markup, and the runtime has
 * already cut them to a label's length.
 *
 * The field is a field and not a composer. One line, one question, and it
 * disappears when the question is answered. See the note at the top of this
 * file about what this surface must never become.
 */
function Answers({
  options,
  disabled,
  onAnswer,
}: {
  options: string[];
  disabled: boolean;
  onAnswer: (answer: string) => void;
}) {
  const [written, setWritten] = useState("");

  if (options.length > 0) {
    return (
      <>
        {options.map((option) => (
          <button
            key={option}
            type="button"
            className="btn btn--small"
            disabled={disabled}
            onClick={() => onAnswer(option)}
          >
            {option}
          </button>
        ))}
      </>
    );
  }

  const send = (event: FormEvent) => {
    event.preventDefault();
    // An empty answer settles the request with nothing in it, and the agent
    // resumes as though it had been told something. The runtime refuses one
    // too; this is the half that stops the operator seeing an error for a
    // press that was obviously a mistake.
    const text = written.trim();
    if (!text) return;
    onAnswer(text);
  };

  return (
    <form className="desk__answer" onSubmit={send}>
      <input
        type="text"
        className="desk__field"
        value={written}
        disabled={disabled}
        placeholder="Your answer"
        onChange={(event) => setWritten(event.target.value)}
      />
      <button
        type="submit"
        className="btn btn--primary btn--small"
        disabled={disabled || written.trim() === ""}
      >
        Send
      </button>
    </form>
  );
}
