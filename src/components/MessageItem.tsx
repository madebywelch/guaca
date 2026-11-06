import { memo } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { api } from "../lib/ipc";
import { routineTitle } from "../lib/routine";
import { useStore } from "../lib/store";
import { whenLabel } from "../lib/time";
import { type Step, trailStep } from "../lib/trail";
import type { Lookups } from "../lib/transcript";
import type {
  AgentCard,
  AgentId,
  Envelope,
  MessageId,
  Part,
  Participant,
  RoutineId,
} from "../lib/types";
import { ApprovalRequest } from "./ApprovalRequest";
import { ArtifactCard } from "./Artifacts";
import { FileCard } from "./FileCard";
import { Markdown } from "./Markdown";
import { QuestionRequest } from "./QuestionRequest";
import { TrailFootnote } from "./Trail";
import { AsideRow, NoticeRow, RoutineRow } from "./WireRow";

interface Props {
  message: Envelope;
  lookups: Lookups;
  /** Hides the margin and the running head when the previous message had the same author. */
  continued: boolean;
  /**
   * Which footnote in the channel this row is, where it is one.
   *
   * A row that only did things — the tool calls of a turn, a burst of peer
   * traffic — is set as a numbered note under the turn it belongs to rather
   * than as a turn of its own. The number is the channel's, assigned in the
   * order the footnotes are drawn, so it is passed in rather than counted
   * here: this component sees one row and a footnote is a position in a page.
   */
  footnote?: number;
}

const HUMAN = { id: "human", name: "You", color: "var(--muted)", avatar: "plain" };
const SYSTEM = { id: "system", name: "Guaca", color: "var(--system)", avatar: "plain" };

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

/**
 * When a stretch of conversation started, drawn between two of them.
 *
 * The transcript's clock, moved off every message and onto the few places it
 * changes anything. What it replaces was a time above four consecutive
 * messages written in the same minute.
 */
export function WhenRow({ at }: { at: number }) {
  return (
    <div className="when-row">
      <time dateTime={new Date(at).toISOString()}>{whenLabel(at, Date.now())}</time>
    </div>
  );
}

/** Keys tied to the message id. A persisted message is immutable. */
function keyed(message: Envelope) {
  return message.parts.map((part, position) => ({ part, key: `${message.id}:${position}` }));
}

/**
 * Opens the routine a fired-routine chip stands for.
 *
 * Imperative for the same reason as {@link onRetry}: the panel that draws a
 * routine is a sibling of the transcript, and the store is where the two meet.
 */
function onOpenRoutine(routineId: RoutineId) {
  useStore.getState().openRoutine(routineId);
}

/**
 * Sends a failed turn's message again.
 *
 * Imperative rather than a prop threaded through every message: this is one
 * button on one kind of notice, and the store is reachable without a hook.
 */
function onRetry(agentId: AgentId, messageId: MessageId) {
  void api.retryTurn(agentId, messageId).catch((error) => {
    useStore.getState().showFailure(error);
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
 * Memoized because a transcript is redrawn whenever any message is appended,
 * and rendering one of these parses its markdown. Thirty messages re-parsed
 * every time an agent says anything is work nobody asked for: the envelopes
 * themselves never change, so an entry that is already on screen is already
 * correct.
 */
export const MessageItem = memo(function MessageItem({
  message,
  lookups,
  continued,
  footnote,
}: Props) {
  const { from, to } = message;

  // Ahead of everything else: a request for permission is addressed to the
  // operator whoever the envelope says it is from, and it is the one thing in a
  // transcript they are expected to act on rather than read.
  const asking = message.parts.find((part) => part.type === "approval");
  if (asking) {
    const askerId = to.kind === "agent" ? to.id : message.channelId;
    return <ApprovalRequest part={asking} agent={lookups.byId(askerId)} />;
  }

  // The other thing an agent stops to ask, and its own card because it is a
  // different question: this one is not asking to be allowed anything.
  const wondering = message.parts.find((part) => part.type === "question");
  if (wondering) {
    const askerId = to.kind === "agent" ? to.id : message.channelId;
    return <QuestionRequest part={wondering} agent={lookups.byId(askerId)} />;
  }

  // A schedule firing. Drawn as one line rather than as the several sentences
  // it carries: the instruction is written for the agent to act on with no
  // other context, and as a bubble it read as Guaca talking to the operator.
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

  const answered = message.parts.find((part) => part.type === "decision");
  if (answered) {
    return <DecisionReply part={answered} at={message.createdAt} />;
  }

  if (from.kind === "agent" && to.kind === "system") {
    return (
      <ActivityRecord
        message={message}
        byId={lookups.byId}
        continued={continued}
        footnote={footnote}
      />
    );
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
 *
 * Tool calls are drawn as one footnote per run of them rather than one row
 * each: a turn can make two dozen, and a line apiece buried the operator's own
 * conversation exactly as peer traffic did before it was collapsed. A notice
 * ends the run it interrupts, for the reason a tool trail ends a burst in
 * `transcriptRows`: the notice is usually about the calls either side of it,
 * and folding across it would put the explanation somewhere other than the
 * thing it explains.
 *
 * Text reaches here in one case: a turn that answered its peer through
 * `send_message` and then carried on typing. That text has no recipient, so it
 * is drawn as an aside rather than as anything addressed to the operator.
 *
 * A `json` part is the one kind that reaches here and draws nothing, and it is
 * still the case that nothing in the runtime emits one. It used to be described
 * here as the seam a model-authored artifact would arrive through; artifacts
 * arrived, and they came through the text instead. A chart or a page is a
 * fenced block in the reply, which costs no round trip and needs no new part:
 * `lib/figure.ts`. A renderer for `json` before there is a producer is still
 * surface with no caller.
 */
function ActivityRecord({
  message,
  byId,
  continued,
  footnote,
}: {
  message: Envelope;
  byId: Lookups["byId"];
  continued: boolean;
  footnote?: number;
}) {
  const blocks: React.ReactNode[] = [];
  /** What is waiting to be drawn as one note, and the key it is filed under. */
  let run: { steps: Step[]; key: string } | null = null;
  /**
   * Whether this row produced something the operator will open.
   *
   * A kept page is work, and work gets a turn: the agent's face, its name and
   * the card under them. Everything else a record holds — calls, notices, a
   * paragraph it trailed after answering somebody else — is apparatus, and
   * apparatus is set as a note or a line rather than as somebody speaking.
   */
  let produced = false;

  const close = () => {
    if (run) {
      blocks.push(
        <TrailFootnote
          key={run.key}
          steps={run.steps}
          number={footnote ?? 0}
          // The turn's own clock, where the note is the whole row. Inside a
          // turn the passage above it already carries the time.
          at={
            message.parts.every((part) => part.type === "toolCall") ? message.createdAt : undefined
          }
        />,
      );
    }
    run = null;
  };

  for (const { part, key } of keyed(message)) {
    // A call that wrote a kept page ends the run and is drawn as a card. It is
    // the one call whose result is a thing the operator will open, and folded
    // into a footnote it would be a page they never learn was made.
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
    // Nothing the operator will open: this is the apparatus of a turn rather
    // than a turn, and it is set as notes and lines under the one before it.
    return <>{blocks}</>;
  }

  const author = identity(message.from, byId);
  return (
    <article className={continued ? "msg msg--continued" : "msg"}>
      <div className="msg__margin">
        {!continued && (
          <time className="msg__at" dateTime={new Date(message.createdAt).toISOString()}>
            {clockTime(message.createdAt)}
          </time>
        )}
        {!continued && (
          <AgentAvatar
            avatar={author.avatar}
            color={author.color}
            size="sm"
            seed={author.id}
            title={author.name}
          />
        )}
      </div>
      <div className="msg__body">{blocks}</div>
    </article>
  );
}

/**
 * The operator's answer to a decision, in the channel of the agent that asked.
 *
 * Guaca delivers it, and it was drawn as Guaca's bubble: the paragraph telling
 * the agent what to do with an answer, then the question and the answer as a
 * line of JSON. What the operator did was answer a question, so it is drawn
 * as their reply, on their side of the column, under the question it answers.
 * What else the agent asked with is behind the question.
 *
 * Every word of the request is the agent's, and so is an answer picked from
 * its options, so all of it is drawn as text and none of it as markdown.
 */
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
      <div className="msg__body">
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
      </div>
      <time className="msg__at" dateTime={new Date(at).toISOString()}>
        {clockTime(at)}
      </time>
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
      {/* The margin: when, and who. The clock used to be a lane on the right
          edge that appeared on hover, which put a message's time as far from
          its first word as the page allowed and hid it besides. A record has
          its date in the margin, where a reader looks for it once. */}
      <div className="msg__margin">
        {!continued && (
          <time className="msg__at" dateTime={new Date(message.createdAt).toISOString()}>
            {clockTime(message.createdAt)}
          </time>
        )}
        {!isOperator && !continued && (
          <AgentAvatar
            avatar={author.avatar}
            color={author.color}
            size="sm"
            seed={author.id}
            title={author.name}
          />
        )}
      </div>

      <div className="msg__body">
        {/* The running head: who is speaking, set in the display face the way a
            printed page sets it, and drawn on the first message of a run
            rather than on every one. */}
        {!continued && <div className="msg__head">{author.name}</div>}

        {notices.map(({ part, key }) => (
          <NoticeRow
            key={key}
            kind={part.kind}
            text={part.text}
            // Only where there is something to send again: the notice records
            // which message the turn was answering when the call failed.
            onRetry={
              (part.kind === "upstreamError" || part.kind === "interrupted") && message.cause
                ? () => onRetry(message.channelId, message.cause as MessageId)
                : undefined
            }
          />
        ))}

        {texts.map(({ part, key }) => (
          <Markdown key={key}>{part.text}</Markdown>
        ))}

        {files.map(({ part, key }) => (
          <FileCard key={key} file={part} />
        ))}
      </div>
    </article>
  );
}

/**
 * The in-progress bubble shown while an agent is composing.
 *
 * Only ever drawn in that agent's own channel, so it carries no name and no
 * clock: the caret is what says this one is still being written, and it will
 * settle into a bubble of exactly this shape.
 */
export function StreamingMessage({ agent, text }: { agent: AgentCard | undefined; text: string }) {
  return (
    <article className="msg">
      <div className="msg__gutter">
        <AgentAvatar
          avatar={agent?.avatar ?? "orb"}
          color={agent?.color ?? "#5a7d99"}
          size="sm"
          seed={agent?.id}
          activity={{ state: "thinking" }}
          title={agent?.name}
        />
      </div>
      <div className="msg__body md--streaming">
        {/* Still being written, which is the one thing a body needs to know
            about itself: a page is framed once, whole, rather than reloaded
            on every token that lands. */}
        <Markdown live>{text}</Markdown>
      </div>
    </article>
  );
}
