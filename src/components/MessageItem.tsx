import { AgentAvatar } from "../avatars/AgentAvatar";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { sendBody, sendRecipients } from "../lib/toolArgs";
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
import { NoticeRow, toPeer, WireRow } from "./WireRow";

export interface Lookups {
  byId: (id: AgentId) => AgentCard | undefined;
  /** Tool calls name their recipients, so names need resolving too. */
  byName: (name: string) => AgentCard | undefined;
}

interface Props {
  message: Envelope;
  lookups: Lookups;
  /** Hides the avatar and header when the previous message had the same author. */
  continued: boolean;
  /** The activity feed shows both ends of a message; a channel shows one. */
  feed: boolean;
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

function plainBody(message: Envelope): string {
  return message.parts
    .filter((p): p is Extract<Part, { type: "text" }> => p.type === "text")
    .map((p) => p.text)
    .join("\n")
    .trim();
}

/**
 * One entry in a transcript.
 *
 * Three shapes, chosen by who was talking to whom:
 *
 * - operator to agent, or agent to operator: a chat bubble. These are the only
 *   messages written to be read in full.
 * - agent to agent: a single centred line with the content one click away.
 *   Rendering these as bubbles buried the operator's own conversation under
 *   machine chatter they were never meant to read line by line.
 * - agent to system: that agent's own record of what it did on its turn.
 */
export function MessageItem({ message, lookups, continued, feed }: Props) {
  const { from, to } = message;

  // Ahead of everything else: a request for permission is addressed to the
  // operator whoever the envelope says it is from, and it is the one thing in a
  // transcript they are expected to act on rather than read.
  const asking = message.parts.find((part) => part.type === "approval");
  if (asking) {
    const askerId = to.kind === "agent" ? to.id : message.channelId;
    return <ApprovalRequest part={asking} agent={lookups.byId(askerId)} />;
  }

  if (from.kind === "agent" && to.kind === "agent") {
    return (
      <WireRow
        direction={feed ? "between" : "received"}
        peer={toPeer(lookups.byId(from.id), from.id)}
        counterpart={toPeer(lookups.byId(to.id), to.id)}
        hop={message.hop}
        at={message.createdAt}
        body={plainBody(message)}
      />
    );
  }

  if (from.kind === "agent" && to.kind === "system") {
    return <ActivityRecord message={message} lookups={lookups} />;
  }

  return <ChatBubble message={message} byId={lookups.byId} continued={continued} />;
}

/** What an agent did on its own turn: who it wrote to, and any guard stops. */
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

        // Everything that is not a send is a quiet one-liner. This used to fall
        // through to the send renderer, so `update_notes` — which has no
        // recipients — was drawn as "Sent to no one" with the memory body as
        // the message. Naming the tools that are not sends is what stops the
        // next one from doing the same.
        if (part.name !== "send_message") {
          const summary =
            part.outcome.status === "ok" || part.outcome.status === "partial"
              ? part.outcome.summary
              : "";
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
                {summary ? ` — ${summary}` : ""}
              </span>
            </div>
          );
        }

        const names = sendRecipients(part.arguments);
        const text = sendBody(part.arguments);
        const outcome = part.outcome;
        // Per recipient, because a fan-out can be half-delivered. Painting one
        // verdict across every row drew agents that were refused as "Sent to",
        // which is the one thing this trail exists to be right about.
        const refusalFor = (name: string): string | null => {
          if (outcome.status === "refused") return outcome.reason;
          if (outcome.status === "failed") return outcome.error;
          if (outcome.status === "partial") {
            return outcome.refused.find((r) => r.to === name)?.reason ?? null;
          }
          return null;
        };

        // One row per recipient: "sent to three agents" hides which three.
        const targets = names.length > 0 ? names : ["no one"];
        return targets.map((name) => (
          <WireRow
            key={`${key}:${name}`}
            direction="sent"
            peer={toPeer(lookups.byName(name), name, name)}
            at={message.createdAt}
            body={text}
            refusal={refusalFor(name)}
          />
        ));
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
      </div>
    </article>
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
