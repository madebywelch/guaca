import { type FormEvent, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { useStore } from "../lib/store";
import { relativeTime } from "../lib/time";
import type { AgentCard, Approval, Decision, Escalation } from "../lib/types";

/** Live permissions and operational blockers inside For you. */
export function Desk() {
  const pending = useStore((s) => s.pending);
  const stuck = useStore((s) => s.stuck);
  const agents = useStore((s) => s.agents);
  const select = useStore((s) => s.select);
  const decide = useStore((s) => s.decideApproval);
  const answer = useStore((s) => s.answerQuestion);
  const clear = useStore((s) => s.clearEscalation);
  const open = (id: string) => {
    void select(id);
    useStore.getState().showForYou(false);
  };
  if (pending.length + stuck.length === 0) return null;
  return (
    <section className="for-you__requests" aria-label="Waiting on you">
      <h3>{deskSummary(pending.length, stuck.length)}</h3>
      {pending.map((request) => (
        <DeskCard
          key={request.id}
          request={request}
          agent={agents.find((a) => a.id === request.agentId)}
          onDecide={(decision) => decide(request.id, decision)}
          onAnswer={(text) => answer(request.id, text)}
          onOpenChannel={() => open(request.agentId)}
        />
      ))}
      {stuck.map((one) => (
        <StuckCard
          key={one.id}
          escalation={one}
          agent={agents.find((a) => a.id === one.agentId)}
          onClear={() => void clear(one.id)}
          onOpenChannel={() => open(one.agentId)}
        />
      ))}
    </section>
  );
}
export function deskSummary(waiting: number, stuck: number): string {
  if (stuck === 0)
    return waiting === 1 ? "1 turn is waiting on you" : `${waiting} turns are waiting on you`;
  if (waiting === 0)
    return stuck === 1 ? "1 agent needs your help" : `${stuck} agents need your help`;
  return `${waiting + stuck} things are waiting on you`;
}

interface StuckProps {
  escalation: Escalation;
  agent: AgentCard | undefined;
  onClear: () => void;
  onOpenChannel: () => void;
}

/**
 * One escalation, and the two things that can be done about it.
 *
 * Open is the primary and Clear is not, which is the whole card. Clearing takes
 * the row away and changes nothing about the agent: what actually unblocks it
 * is a message in the channel, and an operator who only ever presses the
 * cheaper button is running a desk they tidy instead of a desk they work. So
 * the button that leads somewhere is the loud one, and the one that ends it
 * quietly is the quiet one.
 *
 * The age and the count are on the card rather than in the summary because they
 * are the news. "Stuck" is a state; "stuck since Tuesday, six turns into it" is
 * a decision. Neither number can be recovered from the transcript without
 * reading a week of it.
 */
function StuckCard({ escalation, agent, onClear, onOpenChannel }: StuckProps) {
  const asker = agent?.name ?? "A deleted agent";
  const age = relativeTime(escalation.raisedAt, Date.now());
  const turns = escalation.times === 1 ? "1 turn" : `${escalation.times} turns`;

  return (
    <article className="desk__card" data-kind="stuck">
      <header className="desk__who">
        <AgentAvatar
          avatar={agent?.avatar ?? "blank"}
          color={agent?.color ?? "#8aa0a6"}
          size="sm"
          seed={agent?.id ?? escalation.id}
        />
        <div className="desk__whotext">
          <p className="desk__asker">
            {asker} · stuck {age}
          </p>
          {/* The agent's own words, drawn as text under a heading Guaca wrote,
              exactly as a request's detail is. */}
          <p className="desk__summary">{escalation.summary}</p>
        </div>
      </header>

      <div className="desk__actions">
        <button type="button" className="btn btn--primary btn--small" onClick={onOpenChannel}>
          Open channel
        </button>
        <button type="button" className="btn btn--ghost btn--small" onClick={onClear}>
          Clear
        </button>
        <span className="desk__aside">{turns} have run into this</span>
      </div>
    </article>
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
