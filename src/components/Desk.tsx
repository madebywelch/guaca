import { type FormEvent, type ReactNode, useEffect, useReducer, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { ABSENT_ACCENT } from "../avatars/catalog";
import { useStore } from "../lib/store";
import { clockLabel, relativeTime } from "../lib/time";
import type { AgentCard, AgentId, Approval, Decision, Escalation } from "../lib/types";

export function Desk({
  onOpenChannel,
  title,
}: {
  onOpenChannel: (agent: AgentId) => void;
  title?: ReactNode;
}) {
  const pending = useStore((s) => s.pending);
  const stuck = useStore((s) => s.stuck);
  const agents = useStore((s) => s.agents);
  const decide = useStore((s) => s.decideApproval);
  const answer = useStore((s) => s.answerQuestion);
  const clear = useStore((s) => s.clearEscalation);
  if (pending.length + stuck.length === 0) return null;
  return (
    <section className="for-you__requests" aria-label="Waiting on you">
      <h3>{title ?? deskSummary(pending.length, stuck.length)}</h3>
      {pending.map((request) => (
        <DeskCard
          key={request.id}
          request={request}
          agent={agents.find((a) => a.id === request.agentId)}
          onDecide={(decision) => decide(request.id, decision)}
          onAnswer={(text) => answer(request.id, text)}
          onOpenChannel={() => onOpenChannel(request.agentId)}
        />
      ))}
      {stuck.map((one) => (
        <StuckCard
          key={one.id}
          escalation={one}
          agent={agents.find((a) => a.id === one.agentId)}
          onClear={() => void clear(one.id)}
          onOpenChannel={() => onOpenChannel(one.agentId)}
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

function StuckCard({ escalation, agent, onClear, onOpenChannel }: StuckProps) {
  const asker = agent?.name ?? "A deleted agent";
  const age = relativeTime(escalation.raisedAt, Date.now());
  const turns = escalation.times === 1 ? "1 turn has" : `${escalation.times} turns have`;

  return (
    <article className="desk__card" data-kind="stuck">
      <header className="desk__who">
        <AgentAvatar
          avatar={agent?.avatar ?? "blank"}
          color={agent?.color ?? ABSENT_ACCENT}
          size="sm"
          seed={agent?.id ?? escalation.id}
        />
        <div className="desk__whotext">
          <p className="desk__asker">
            {asker} · stuck {age}
          </p>
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
        <span className="desk__aside">{turns} run into this</span>
      </div>
    </article>
  );
}

interface CardProps {
  request: Approval;
  agent: AgentCard | undefined;
  onDecide: (decision: Decision) => Promise<void>;
  onAnswer: (answer: string) => Promise<void>;
  onOpenChannel: () => void;
}

function DeskCard({ request, agent, onDecide, onAnswer, onOpenChannel }: CardProps) {
  const [answering, setAnswering] = useState(false);
  const [, lapsed] = useReducer((n: number) => n + 1, 0);
  useEffect(() => {
    if (request.lapsesAt === null) return;
    const left = request.lapsesAt - Date.now();
    if (left <= 0) return;
    const timer = window.setTimeout(lapsed, left);
    return () => window.clearTimeout(timer);
  }, [request.lapsesAt]);
  const asker = agent?.name ?? "A deleted agent";

  const answer = (decision: Decision) => {
    setAnswering(true);
    void onDecide(decision).finally(() => setAnswering(false));
  };

  return (
    <article className="desk__card">
      <header className="desk__who">
        <AgentAvatar
          avatar={agent?.avatar ?? "blank"}
          color={agent?.color ?? ABSENT_ACCENT}
          size="sm"
          seed={agent?.id ?? request.id}
        />
        <div className="desk__whotext">
          <p className="desk__asker">
            {asker} <span className="desk__when">asked at {clockLabel(request.createdAt)}</span>
          </p>
          <p className="desk__summary">{request.summary}</p>
        </div>
        <button type="button" className="desk__open" onClick={onOpenChannel}>
          Open channel
        </button>
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
        {request.lapsesAt !== null && request.lapsesAt > Date.now() && (
          <span className="desk__aside">
            {asker} waits until {clockLabel(request.lapsesAt)}
          </span>
        )}
      </div>
    </article>
  );
}

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
        {options.map((option, index) => (
          <button
            key={option}
            type="button"
            className={index === 0 ? "btn btn--primary btn--small" : "btn btn--ghost btn--small"}
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
