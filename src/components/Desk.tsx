import {
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useReducer,
  useRef,
  useState,
} from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { ABSENT_ACCENT } from "../avatars/catalog";
import { useHeld, useShown } from "../lib/held";
import { formatCombo, IS_MAC } from "../lib/keybinds";
import { askers, whoIsWaiting } from "../lib/questions";
import { useStore } from "../lib/store";
import { clockLabel, relativeTime } from "../lib/time";
import type { AgentCard, AgentId, Approval, Decision, Escalation } from "../lib/types";

export interface Queue {
  selected: string | null;
  onSelect: (id: string) => void;
  onLeave: () => void;
}

const ALLOW = formatCombo({ key: "Enter", mod: true });
const DENY = formatCombo({ key: "Backspace", mod: true });
const MOD = IS_MAC ? "Meta" : "Control";

export function Desk({
  onOpenChannel,
  title,
  queue,
}: {
  onOpenChannel: (agent: AgentId) => void;
  title?: ReactNode | null;
  queue?: Queue;
}) {
  const { pending, stuck } = useShown();
  const agents = useStore((s) => s.agents);
  const decide = useStore((s) => s.decideApproval);
  const answer = useStore((s) => s.answerQuestion);
  const clear = useStore((s) => s.clearEscalation);
  const hold = useHeld((s) => s.hold);
  const order = [...pending.map((request) => request.id), ...stuck.map((one) => one.id)];
  const selected = queue && order.includes(queue.selected ?? "") ? queue.selected : null;

  if (order.length === 0) return null;

  const nameOf = (agentId: AgentId) =>
    agents.find((agent) => agent.id === agentId)?.name ?? "A deleted agent";

  const advance = (id: string) => {
    if (!queue) return;
    const at = order.indexOf(id);
    const next = order[at + 1] ?? order[at - 1];
    if (next === undefined) queue.onLeave();
    else queue.onSelect(next);
  };

  const held = (id: string, text: string, send: () => Promise<void>) => {
    if (!queue) return send();
    const wasSelected = selected === id;
    hold(id, text, send);
    if (wasSelected || selected === null) advance(id);
    return Promise.resolve();
  };

  const decideHeld = (request: Approval, decision: Decision) =>
    held(
      request.id,
      `${decision === "deny" ? "Denied" : "Allowed"} what ${nameOf(request.agentId)} asked.`,
      () => decide(request.id, decision),
    );

  const answerHeld = (request: Approval, text: string) =>
    held(request.id, `Answered ${nameOf(request.agentId)}: ${text}`, () =>
      answer(request.id, text),
    );

  const clearHeld = (one: Escalation) =>
    held(one.id, `Cleared what ${nameOf(one.agentId)} asked.`, () => clear(one.id));

  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    if (!queue || event.nativeEvent.isComposing || event.altKey) return;
    const target = event.target;
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement) return;
    if (
      !(target instanceof HTMLElement) ||
      !target.closest(".desk__card")?.hasAttribute("data-selected")
    )
      return;
    const mod = event.metaKey || event.ctrlKey;
    const at = selected === null ? -1 : order.indexOf(selected);
    const step = (to: number) => {
      const id = order[Math.max(0, Math.min(order.length - 1, to))];
      if (id !== undefined) queue.onSelect(id);
    };
    const key = event.key.toLowerCase();
    if (!mod && (key === "j" || event.key === "ArrowDown")) {
      event.preventDefault();
      step(at + 1);
      return;
    }
    if (!mod && (key === "k" || event.key === "ArrowUp")) {
      event.preventDefault();
      step(at <= 0 ? 0 : at - 1);
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      queue.onLeave();
      return;
    }
    const request = pending.find((one) => one.id === selected);
    const escalation = stuck.find((one) => one.id === selected);
    if (mod && event.key === "Enter") {
      event.preventDefault();
      if (request?.request.kind === "permission") {
        void decideHeld(request, "allow");
      } else if (escalation) {
        onOpenChannel(escalation.agentId);
      }
      return;
    }
    if (mod && event.key === "Backspace") {
      event.preventDefault();
      if (request?.request.kind === "permission") {
        void decideHeld(request, "deny");
      } else if (escalation) {
        void clearHeld(escalation);
      }
      return;
    }
    if (!mod && !event.shiftKey && /^[1-9]$/.test(event.key)) {
      const option =
        request?.request.kind === "question"
          ? request.request.options[Number(event.key) - 1]
          : undefined;
      if (request && option !== undefined) {
        event.preventDefault();
        void answerHeld(request, option);
      }
    }
  };

  return (
    <section className="for-you__requests" aria-label="Waiting on you" onKeyDown={onKeyDown}>
      {title !== null && <h3>{title ?? whoIsWaiting(askers([...pending, ...stuck], nameOf))}</h3>}
      {pending.map((request) => (
        <DeskCard
          key={request.id}
          request={request}
          agent={agents.find((a) => a.id === request.agentId)}
          selected={selected === request.id}
          onSelect={queue ? () => queue.onSelect(request.id) : undefined}
          onDecide={(decision) => decideHeld(request, decision)}
          onAnswer={(text) => answerHeld(request, text)}
          onOpenChannel={() => onOpenChannel(request.agentId)}
        />
      ))}
      {stuck.map((one) => (
        <StuckCard
          key={one.id}
          escalation={one}
          agent={agents.find((a) => a.id === one.agentId)}
          selected={selected === one.id}
          onSelect={queue ? () => queue.onSelect(one.id) : undefined}
          onClear={() => void clearHeld(one)}
          onOpenChannel={() => onOpenChannel(one.agentId)}
        />
      ))}
    </section>
  );
}

function Key({ shown, label }: { shown: boolean; label: string }) {
  if (!shown) return null;
  return (
    <span className="desk__key" aria-hidden="true">
      {label}
    </span>
  );
}

function useSelectedFocus(selected: boolean) {
  const card = useRef<HTMLElement>(null);
  useEffect(() => {
    if (!selected) return;
    const node = card.current;
    if (!node) return;
    if (!node.contains(document.activeElement)) node.focus({ preventScroll: true });
    node.scrollIntoView?.({ block: "nearest" });
  }, [selected]);
  return card;
}

interface Selectable {
  selected: boolean;
  onSelect?: () => void;
}

interface StuckProps extends Selectable {
  escalation: Escalation;
  agent: AgentCard | undefined;
  onClear: () => void;
  onOpenChannel: () => void;
}

function StuckCard({ escalation, agent, selected, onSelect, onClear, onOpenChannel }: StuckProps) {
  const card = useSelectedFocus(selected);
  const asker = agent?.name ?? "A deleted agent";
  const age = relativeTime(escalation.raisedAt, Date.now());
  const turns = escalation.times === 1 ? "1 turn has" : `${escalation.times} turns have`;

  return (
    <article
      ref={card}
      className="desk__card"
      data-kind="stuck"
      data-selected={selected ? "" : undefined}
      tabIndex={onSelect ? -1 : undefined}
      aria-label={onSelect ? `${asker} is stuck: ${escalation.summary}` : undefined}
      onFocus={onSelect}
    >
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
        <button
          type="button"
          className="btn btn--primary btn--compact"
          aria-keyshortcuts={onSelect ? `${MOD}+Enter` : undefined}
          onClick={onOpenChannel}
        >
          Open conversation
          <Key shown={selected} label={ALLOW} />
        </button>
        <button
          type="button"
          className="btn btn--quiet btn--compact"
          aria-keyshortcuts={onSelect ? `${MOD}+Backspace` : undefined}
          onClick={onClear}
        >
          Clear
          <Key shown={selected} label={DENY} />
        </button>
        <span className="desk__aside">{turns} run into this</span>
      </div>
    </article>
  );
}

interface CardProps extends Selectable {
  request: Approval;
  agent: AgentCard | undefined;
  onDecide: (decision: Decision) => Promise<void>;
  onAnswer: (answer: string) => Promise<void>;
  onOpenChannel: () => void;
}

function DeskCard({
  request,
  agent,
  selected,
  onSelect,
  onDecide,
  onAnswer,
  onOpenChannel,
}: CardProps) {
  const card = useSelectedFocus(selected);
  const [answering, setAnswering] = useState<string | null>(null);
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
    setAnswering(decision);
    void onDecide(decision).finally(() => setAnswering(null));
  };

  return (
    <article
      ref={card}
      className="desk__card"
      data-selected={selected ? "" : undefined}
      tabIndex={onSelect ? -1 : undefined}
      aria-label={onSelect ? `${asker} asks: ${request.summary}` : undefined}
      onFocus={onSelect}
    >
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
          Open conversation
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
              className="btn btn--primary"
              disabled={answering !== null}
              aria-busy={answering === "allow"}
              aria-keyshortcuts={onSelect ? `${MOD}+Enter` : undefined}
              onClick={() => answer("allow")}
            >
              Allow
              <Key shown={selected} label={ALLOW} />
            </button>
            {request.request.action === "createAgent" && (
              <button
                type="button"
                className="btn"
                disabled={answering !== null}
                aria-busy={answering === "alwaysAllow"}
                title={`Stop asking when ${asker} does this`}
                onClick={() => answer("alwaysAllow")}
              >
                Always
              </button>
            )}
            <button
              type="button"
              className="btn btn--quiet"
              disabled={answering !== null}
              aria-busy={answering === "deny"}
              aria-keyshortcuts={onSelect ? `${MOD}+Backspace` : undefined}
              onClick={() => answer("deny")}
            >
              Deny
              <Key shown={selected} label={DENY} />
            </button>
          </>
        ) : (
          <Answers
            options={request.request.options}
            pending={answering}
            keyed={selected}
            onAnswer={(text) => {
              setAnswering(text);
              void onAnswer(text).finally(() => setAnswering(null));
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
  pending,
  keyed,
  onAnswer,
}: {
  options: string[];
  pending: string | null;
  keyed: boolean;
  onAnswer: (answer: string) => void;
}) {
  const [written, setWritten] = useState("");
  const disabled = pending !== null;

  if (options.length > 0) {
    return (
      <>
        {options.map((option, index) => (
          <button
            key={option}
            type="button"
            className="btn"
            disabled={disabled}
            aria-busy={pending === option}
            aria-keyshortcuts={keyed && index < 9 ? String(index + 1) : undefined}
            onClick={() => onAnswer(option)}
          >
            {option}
            <Key shown={keyed && index < 9} label={String(index + 1)} />
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
        className="btn btn--primary"
        disabled={disabled || written.trim() === ""}
        aria-busy={disabled}
      >
        Send
      </button>
    </form>
  );
}
