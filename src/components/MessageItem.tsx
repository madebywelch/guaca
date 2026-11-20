import { memo, useId, useLayoutEffect, useRef, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { ABSENT_ACCENT } from "../avatars/catalog";
import { api } from "../lib/ipc";
import { routineTitle } from "../lib/routine";
import { useStore } from "../lib/store";
import { whenLabel } from "../lib/time";
import { type Step, trailStep } from "../lib/trail";
import type { Lookups, Row } from "../lib/transcript";
import type {
  AgentCard,
  AgentId,
  Envelope,
  MessageId,
  Part,
  Participant,
  RoutineId,
} from "../lib/types";
import { TIDY_LABEL } from "../lib/types";
import { ApprovalRequest } from "./ApprovalRequest";
import { ArtifactCard } from "./Artifacts";
import { FileCard } from "./FileCard";
import { Markdown } from "./Markdown";
import { QuestionRequest } from "./QuestionRequest";
import { TrailFootnote } from "./Trail";
import { AsideRow, HandoffRecord, NoticeRow, RoutineRow } from "./WireRow";

interface Props {
  message: Envelope;
  lookups: Lookups;
  continued: boolean;
}

const HUMAN = { id: "human", name: "You", color: "var(--muted)", avatar: "plain" };
const SYSTEM = { id: "system", name: "Guaca", color: "var(--system)", avatar: "plain" };

function identity(participant: Participant, byId: Lookups["byId"]) {
  if (participant.kind === "human") return HUMAN;
  if (participant.kind === "system") return SYSTEM;
  const card = byId(participant.id);
  return card
    ? { id: card.id, name: card.name, color: card.color, avatar: card.avatar }
    : { id: participant.id, name: "Deleted agent", color: ABSENT_ACCENT, avatar: "blank" };
}

function clockTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function WhenRow({ at }: { at: number }) {
  return (
    <div className="when-row">
      <time dateTime={new Date(at).toISOString()}>{whenLabel(at, Date.now())}</time>
    </div>
  );
}

function keyed(message: Envelope) {
  return message.parts.map((part, position) => ({ part, key: `${message.id}:${position}` }));
}

function onOpenRoutine(routineId: RoutineId) {
  useStore.getState().openRoutine(routineId);
}

function onRetry(agentId: AgentId, messageId: MessageId) {
  void api.retryTurn(agentId, messageId).catch((error) => {
    useStore.getState().showFailure(error);
  });
}

export const MessageItem = memo(function MessageItem({ message, lookups, continued }: Props) {
  const { from, to } = message;

  const asking = message.parts.find((part) => part.type === "approval");
  if (asking) {
    const askerId = to.kind === "agent" ? to.id : message.channelId;
    return <ApprovalRequest part={asking} agent={lookups.byId(askerId)} />;
  }

  const wondering = message.parts.find((part) => part.type === "question");
  if (wondering) {
    const askerId = to.kind === "agent" ? to.id : message.channelId;
    return <QuestionRequest part={wondering} agent={lookups.byId(askerId)} />;
  }

  const fired = message.parts.find((part) => part.type === "routine");
  if (fired) {
    return (
      <RoutineRow
        title={routineTitle({ name: fired.name, what: fired.what })}
        what={fired.what}
        payload={fired.payload}
        at={message.createdAt}
        onOpen={() => onOpenRoutine(fired.routineId)}
      />
    );
  }

  if (message.parts.some((part) => part.type === "tidy")) {
    return <TidyRow at={message.createdAt} />;
  }

  const answered = message.parts.find((part) => part.type === "decision");
  if (answered) {
    return <DecisionReply part={answered} at={message.createdAt} />;
  }

  if (from.kind === "agent" && to.kind === "system") {
    return <ActivityRecord message={message} byId={lookups.byId} continued={continued} />;
  }

  return <ChatBubble message={message} byId={lookups.byId} continued={continued} />;
});

function TurnHead({
  author,
  at,
}: {
  author: { id: string; name: string; color: string; avatar: string };
  at: number;
}) {
  return (
    <div className="msg__head">
      <AgentAvatar avatar={author.avatar} color={author.color} size="xs" seed={author.id} />
      <span className="msg__name">{author.name}</span>
      <time className="msg__at" dateTime={new Date(at).toISOString()}>
        {clockTime(at)}
      </time>
    </div>
  );
}

function TidyRow({ at }: { at: number }) {
  return (
    <div className="wire wire--routine">
      <span className="wire__chip">
        <span className="wire__label">{TIDY_LABEL}</span>
        <span className="wire__why">ran</span>
        <span className="wire__meta">{clockTime(at)}</span>
      </span>
    </div>
  );
}

function ActivityRecord({
  message,
  byId,
  continued,
}: {
  message: Envelope;
  byId: Lookups["byId"];
  continued: boolean;
}) {
  const blocks: React.ReactNode[] = [];
  let run: { steps: Step[]; key: string } | null = null;
  let produced = false;

  const close = () => {
    if (run) {
      blocks.push(<TrailFootnote key={run.key} steps={run.steps} />);
    }
    run = null;
  };

  for (const { part, key } of keyed(message)) {
    if (part.type === "toolCall" && part.artifact) {
      close();
      produced = true;
      blocks.push(<ArtifactCard key={key} made={part.artifact} />);
      continue;
    }
    if (part.type === "toolCall") {
      const step = trailStep(part, key);
      if (run) run.steps.push(step);
      else run = { steps: [step], key };
      continue;
    }
    close();
    if (part.type === "notice") {
      blocks.push(<NoticeRow key={key} kind={part.kind} text={part.text} />);
    }
    if (part.type === "text") {
      blocks.push(<AsideRow key={key} text={part.text} />);
    }
  }
  close();

  if (!produced) {
    return <>{blocks}</>;
  }

  const author = identity(message.from, byId);
  return (
    <article className={continued ? "msg msg--continued" : "msg"}>
      {!continued && <TurnHead author={author} at={message.createdAt} />}
      <div className="msg__body">{blocks}</div>
    </article>
  );
}

function DecisionReply({ part, at }: { part: Extract<Part, { type: "decision" }>; at: number }) {
  const { request } = part;
  const more = (
    [
      ["Context", request.context],
      ["Recommended", request.recommendation],
      ["Source", request.source],
    ] as const
  ).filter(([, text]) => text !== "");

  return (
    <article className="msg" data-operator="true">
      <div className="msg__bubble">
        <p className="decision__label">
          {part.resumed ? "Resumed in For you" : "Answered in For you"}
        </p>
        {more.length > 0 ? (
          <details className="decision__asked">
            <summary title="Show what it was asked with">{request.question}</summary>
            <dl className="decision__more">
              {more.map(([label, text]) => (
                <div key={label}>
                  <dt>{label}</dt>
                  <dd>{text}</dd>
                </div>
              ))}
            </dl>
          </details>
        ) : (
          <p className="decision__asked">{request.question}</p>
        )}
        <p className="decision__answer">{part.answer}</p>
        <time className="msg__at" dateTime={new Date(at).toISOString()}>
          {clockTime(at)}
        </time>
      </div>
    </article>
  );
}

function ChatBubble({
  message,
  byId,
  continued,
}: {
  message: Envelope;
  byId: Lookups["byId"];
  continued: boolean;
}) {
  const author = identity(message.from, byId);
  const isOperator = message.from.kind === "human";

  const parts = keyed(message);
  const texts = parts.filter(
    (e): e is { part: Extract<Part, { type: "text" }>; key: string } => e.part.type === "text",
  );
  const notices = parts.filter(
    (e): e is { part: Extract<Part, { type: "notice" }>; key: string } => e.part.type === "notice",
  );
  const files = parts.filter(
    (e): e is { part: Extract<Part, { type: "file" }>; key: string } => e.part.type === "file",
  );

  return (
    <article
      className={continued ? "msg msg--continued" : "msg"}
      data-operator={isOperator ? "true" : undefined}
      aria-label={isOperator ? `You, ${clockTime(message.createdAt)}` : undefined}
    >
      {!isOperator && !continued && <TurnHead author={author} at={message.createdAt} />}

      <div className={isOperator ? "msg__bubble" : "msg__body"}>
        {notices.map(({ part, key }) => (
          <NoticeRow
            key={key}
            kind={part.kind}
            text={part.text}
            onRetry={
              (part.kind === "upstreamError" || part.kind === "interrupted") && message.cause
                ? () => onRetry(message.channelId, message.cause as MessageId)
                : undefined
            }
          />
        ))}

        {isOperator ? (
          <Brief>
            {texts.map(({ part, key }) => (
              <Markdown key={key}>{part.text}</Markdown>
            ))}
          </Brief>
        ) : (
          texts.map(({ part, key }) => <Markdown key={key}>{part.text}</Markdown>)
        )}

        {files.map(({ part, key }) => (
          <FileCard key={key} file={part} />
        ))}
      </div>
    </article>
  );
}

function Brief({ children }: { children: React.ReactNode }) {
  const body = useRef<HTMLDivElement>(null);
  const region = useId();
  const [open, setOpen] = useState(false);
  const [long, setLong] = useState(false);

  useLayoutEffect(() => {
    const node = body.current;
    if (!node || open) return;
    const measure = () => setLong(node.scrollHeight > node.clientHeight);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [open]);

  return (
    <>
      <div ref={body} id={region} className="msg__brief" data-folded={open ? undefined : "true"}>
        {children}
      </div>
      {(long || open) && (
        <button
          type="button"
          className="msg__more"
          aria-expanded={open}
          aria-controls={region}
          onClick={() => setOpen(!open)}
        >
          {open ? "Show less" : "Show all"}
        </button>
      )}
    </>
  );
}

export function ResultItem({
  row,
  lookups,
  channel,
  open,
  onToggle,
  onOpen,
}: {
  row: Extract<Row, { kind: "result" }>;
  lookups: Lookups;
  channel: string;
  open: boolean;
  onToggle: () => void;
  onOpen: (peer: AgentId) => void;
}) {
  const { message, handoff } = row;
  const author = identity(message.from, lookups.byId);
  const parts = keyed(message);
  return (
    <>
      {handoff && (
        <HandoffRecord
          peers={handoff.peers}
          runId={handoff.runId}
          channel={channel}
          rowKey={handoff.key}
          open={open}
          onToggle={onToggle}
          onOpen={onOpen}
        />
      )}
      <article
        className="msg msg--result"
        aria-label={`Result from ${author.name}, ${clockTime(message.createdAt)}`}
      >
        <div className="result">
          <TurnHead author={author} at={message.createdAt} />
          <div className="result__body">
            {parts.map(({ part, key }) =>
              part.type === "text" ? (
                <Markdown key={key}>{part.text}</Markdown>
              ) : part.type === "file" ? (
                <FileCard key={key} file={part} />
              ) : null,
            )}
          </div>
        </div>
      </article>
    </>
  );
}

export function StreamingMessage({ agent, text }: { agent: AgentCard | undefined; text: string }) {
  return (
    <article className="msg">
      <div className="msg__head">
        <AgentAvatar
          avatar={agent?.avatar ?? "orb"}
          color={agent?.color ?? ABSENT_ACCENT}
          size="xs"
          seed={agent?.id}
          activity={{ state: "thinking" }}
        />
        <span className="msg__name">{agent?.name ?? "Deleted agent"}</span>
      </div>
      <div className="msg__body md--streaming">
        <Markdown live>{text}</Markdown>
      </div>
    </article>
  );
}
