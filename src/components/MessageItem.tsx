import { memo } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type Lookups, toPeer } from "../lib/transcript";
import {
  type AgentCard,
  type AgentId,
  type Envelope,
  errorMessage,
  type MessageId,
  type Part,
  type Participant,
} from "../lib/types";
import { ApprovalRequest } from "./ApprovalRequest";
import { Markdown } from "./Markdown";
import { NoticeRow } from "./WireRow";

interface Props {
  message: Envelope;
  lookups: Lookups;
  /** Hides the avatar and header when the previous message had the same author. */
  continued: boolean;
}

const HUMAN = { id: "human", name: "You", color: "#5b665e", avatar: "plain" };
const SYSTEM = { id: "system", name: "Guaca", color: "#8a5a2f", avatar: "plain" };

function identity(participant: Participant, byId: Lookups["byId"]) {
  if (participant.kind === "human") return HUMAN;
  if (participant.kind === "system") return SYSTEM;
  const card = byId(participant.id);
  return card
    ? { id: card.id, name: card.name, color: card.color, avatar: card.avatar }
    : { id: participant.id, name: "Deleted agent", color: "#8aa0a6", avatar: "blank" };
}

function clockTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** Keys tied to the message id. A persisted message is immutable. */
function keyed(message: Envelope) {
  return message.parts.map((part, position) => ({ part, key: `${message.id}:${position}` }));
}

/**
 * Sends a failed turn's message again.
 *
 * Imperative rather than a prop threaded through every message: this is one
 * button on one kind of notice, and the store is reachable without a hook.
 */
function onRetry(agentId: AgentId, messageId: MessageId) {
  void api.retryTurn(agentId, messageId).catch((error) => {
    useStore.getState().setBanner({ tone: "error", text: errorMessage(error) });
  });
}

/**
 * One entry in a transcript.
 *
 * Three shapes, chosen by who was talking to whom:
 *
 * - operator to agent, or agent to operator: a chat bubble. These are the only
 *   messages written to be read in full.
 * - agent to agent: also a bubble, because the only place these are drawn one
 *   by one is the thread between that pair, which is opened to read them.
 *   Inside a channel they never reach here: `transcriptRows` has already
 *   collapsed them into a burst.
 * - agent to system: that agent's own record of what it did on its turn.
 *
 * Memoised because a transcript is redrawn whenever any message is appended,
 * and rendering one of these parses its markdown. Thirty messages re-parsed
 * every time an agent says anything is work nobody asked for: the envelopes
 * themselves never change, so an entry that is already on screen is already
 * correct.
 */
export const MessageItem = memo(function MessageItem({ message, lookups, continued }: Props) {
  const { from, to } = message;

  // Ahead of everything else: a request for permission is addressed to the
  // operator whoever the envelope says it is from, and it is the one thing in a
  // transcript they are expected to act on rather than read.
  const asking = message.parts.find((part) => part.type === "approval");
  if (asking) {
    const askerId = to.kind === "agent" ? to.id : message.channelId;
    return <ApprovalRequest part={asking} agent={lookups.byId(askerId)} />;
  }

  if (from.kind === "agent" && to.kind === "system") {
    return <ActivityRecord message={message} lookups={lookups} />;
  }

  return <ChatBubble message={message} byId={lookups.byId} continued={continued} />;
});

/**
 * What an agent did on its own turn: guard stops, and the tools it reached for.
 *
 * Sends are not here. They are traffic between two agents however they were
 * made, so `transcriptRows` lifts them out of the record and into the burst
 * row, leaving this the trail around them. A send that named nobody does stay,
 * because there is no peer to file it under and the reason it went nowhere is
 * the whole of what there is to see.
 */
function ActivityRecord({ message, lookups }: { message: Envelope; lookups: Lookups }) {
  const actorId = message.from.kind === "agent" ? message.from.id : "";
  const actor = toPeer(lookups.byId(actorId), actorId);

  return (
    <>
      {keyed(message).map(({ part, key }) => {
        if (part.type === "notice") {
          return <NoticeRow key={key} kind={part.kind} text={part.text} />;
        }
        if (part.type !== "toolCall") return null;

        // Naming the tools rather than describing whatever came back: a tool
        // this does not know about should read as a tool nobody has explained
        // yet, not as a send. `update_notes`, which has no recipients, was once
        // drawn as "Sent to no one" with the memory body as the message.
        const outcome = part.outcome;
        const said =
          outcome.status === "ok" || outcome.status === "partial"
            ? outcome.summary
            : outcome.status === "refused"
              ? outcome.reason
              : outcome.error;
        const what =
          part.name === "directory"
            ? "checked who is available"
            : part.name === "update_notes"
              ? "updated its memory"
              : part.name === "create_agent"
                ? "asked to add an agent"
                : `used ${part.name}`;
        return (
          <div className="wire wire--quiet" key={key}>
            <span className="wire__quiet-text">
              {actor.name} {what}
              {said ? ` — ${said}` : ""}
            </span>
          </div>
        );
      })}
    </>
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
  // The operator does not need a portrait of themselves in their own log; the
  // avatars are there to tell the agents apart.
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
    >
      <div>
        {!continued && !isOperator && (
          <AgentAvatar
            avatar={author.avatar}
            color={author.color}
            size="sm"
            seed={author.id}
            title={author.name}
          />
        )}
      </div>

      <div style={{ minWidth: 0 }}>
        {!continued && (
          <div className="msg__head">
            <span className="msg__author" style={{ color: author.color }}>
              {author.name}
            </span>
            <time className="msg__time" dateTime={new Date(message.createdAt).toISOString()}>
              {clockTime(message.createdAt)}
            </time>
          </div>
        )}

        {notices.map(({ part, key }) => (
          <NoticeRow
            key={key}
            kind={part.kind}
            text={part.text}
            // Only where there is something to send again: the notice records
            // which message the turn was answering when the call failed.
            onRetry={
              part.kind === "upstreamError" && message.cause
                ? () => onRetry(message.channelId, message.cause as MessageId)
                : undefined
            }
          />
        ))}

        {texts.map(({ part, key }) => (
          <Markdown key={key}>{part.text}</Markdown>
        ))}

        {files.map(({ part, key }) => (
          <FileRow key={key} file={part} />
        ))}
      </div>
    </article>
  );
}

/** Sizes the way a person says them, matching what the agent is told. */
function readableSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${bytes} bytes`;
}

/**
 * A file on a message.
 *
 * Named and sized, and that is all: the operator dropped it or watched an agent
 * send it, so what matters here is that it went, not a preview of it.
 */
function FileRow({ file }: { file: Extract<Part, { type: "file" }> }) {
  return (
    <div className="file" title={file.mime}>
      <span className="file__name">{file.name}</span>
      <span className="file__size">{readableSize(file.bytes)}</span>
    </div>
  );
}

/** The in-progress bubble shown while an agent is composing. */
export function StreamingMessage({ agent, text }: { agent: AgentCard | undefined; text: string }) {
  return (
    <article className="msg">
      <div>
        <AgentAvatar
          avatar={agent?.avatar ?? "plain"}
          color={agent?.color ?? "#c7d96b"}
          size="sm"
          seed={agent?.id}
          activity={{ state: "thinking" }}
        />
      </div>
      <div style={{ minWidth: 0 }}>
        <div className="msg__head">
          <span className="msg__author" style={{ color: agent?.color }}>
            {agent?.name ?? "Agent"}
          </span>
          <span className="msg__time">now</span>
        </div>
        <div className="md--streaming">
          <Markdown>{text}</Markdown>
        </div>
      </div>
    </article>
  );
}
