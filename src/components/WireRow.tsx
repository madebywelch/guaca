import { useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { useDismiss } from "../lib/dismiss";

import { type PeerSummary, summaryLabel, type WirePeer } from "../lib/transcript";
import type { AgentId } from "../lib/types";
import { Markdown } from "./Markdown";

/**
 * Agent-to-agent traffic, collapsed.
 *
 * A channel should read as a conversation with one agent. When every message
 * an agent exchanged with its peers is rendered as a full chat bubble, the
 * operator's own conversation is buried under machine chatter they were never
 * meant to read line by line.
 *
 * So a burst of peer traffic becomes a single centered line naming who was
 * talking and how much of it there was, and the exchange itself is one click
 * away as a thread. Full bubbles are reserved for the two things actually
 * addressed to the operator: what they said, and what an agent said back.
 */

/**
 * The readable half of a refusal.
 *
 * Guard refusals are written to be read by a model mid-turn, so they open with
 * "Refused:" and close with what to do instead. On a chip the operator wants
 * the middle: what happened, once, in a few words.
 */
export function why(reason: string): string {
  const body = reason.replace(/^Refused:\s*/i, "");
  const first = body.split(/(?<=\.)\s/)[0] ?? body;
  return first.replace(/\.$/, "");
}

function clockTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

/** One peer's share of a burst, and the thread behind it. */
function PeerChip({ summary, onOpen }: { summary: PeerSummary; onOpen: (peer: AgentId) => void }) {
  const { peer, agentId } = summary;
  const label = summaryLabel(summary);

  // An unresolved name has no thread to open. It still gets a line: an agent
  // that wrote to a name nobody can find is worth seeing.
  return agentId ? (
    <button
      type="button"
      className="footnote__send"
      title={`Read the conversation with ${peer.name}`}
      onClick={() => onOpen(agentId)}
    >
      {label}
    </button>
  ) : (
    <span className="footnote__send">{label}</span>
  );
}

/**
 * The page's apparatus: one numbered line under the turn it belongs to.
 *
 * A record has footnotes, and a footnote is a smaller thing than the passage it
 * annotates: the tool calls a turn made, the messages it exchanged with a
 * peer. Drawn as their own blocks they were a second and third voice in the
 * transcript, and the operator's own conversation read as one entry among
 * them. Set as a note it still says exactly what happened, in the order it
 * happened, and it is skippable.
 *
 * `at` is drawn where the note has one of its own; a note filed under the
 * message that produced it takes the row's clock from that message instead.
 */
export function FootnoteRow({
  at,
  number,
  children,
  failed,
}: {
  at?: number;
  number: number;
  children: React.ReactNode;
  failed?: boolean;
}) {
  return (
    <div className="footnote" data-failed={failed || undefined}>
      <div className="footnote__margin">
        {at !== undefined && (
          <time className="footnote__at" dateTime={new Date(at).toISOString()}>
            {clockTime(at)}
          </time>
        )}
      </div>
      <div className="footnote__body">
        {/* Unnumbered while it is still arriving: a live row has no position in
            the page yet, because the row it will settle into does not exist. */}
        {number > 0 && (
          <span className="footnote__number" aria-hidden="true">
            {number}
          </span>
        )}
        {children}
      </div>
    </div>
  );
}

/**
 * A burst of peer traffic, as one footnote per peer.
 *
 * Per peer rather than "5 messages with 2 agents", for the same reason a
 * fan-out draws one row per recipient: a count that does not name anyone
 * hides the thing the operator opened the channel to find out. It also means
 * every chip has exactly one thread behind it, so a click never has to ask
 * which conversation was meant.
 *
 * What the mockup's footnote adds to the old centered line is the gist: the
 * first thing said in the exchange, in the same line, so the reader can tell
 * whether it is worth opening without opening it. That is a claim about the
 * message and not about the runtime — a burst is still one row per peer, and
 * the count is still of what this channel holds.
 */
export function PeerBurstRow({
  peers,
  number,
  at,
  onOpen,
}: {
  peers: PeerSummary[];
  number: number;
  at: number;
  onOpen: (peer: AgentId) => void;
}) {
  return (
    <FootnoteRow number={number} at={at}>
      <span className="footnote__line">
        {peers.map((summary, at) => (
          <span className="footnote__item" key={summary.peer.id}>
            {at > 0 && (
              <span className="footnote__dot" aria-hidden="true">
                ·
              </span>
            )}
            <PeerChip summary={summary} onOpen={onOpen} />
            {summary.gist && <span className="footnote__gist">{summary.gist}</span>}
          </span>
        ))}
      </span>
    </FootnoteRow>
  );
}

/**
 * A send that did not go.
 *
 * Never folded into a burst, because it is not part of the conversation: it is
 * the runtime stopping one. The reason is on the chip rather than behind the
 * click, since a row of bare "not delivered" chips reads as the app breaking,
 * and the reason is usually a guard doing exactly its job.
 */
export function RefusedRow({
  peer,
  at,
  number,
  body,
  reason,
}: {
  peer: WirePeer;
  at: number;
  number: number;
  body: string;
  reason: string;
}) {
  const [open, setOpen] = useState(false);
  const label = `Not delivered to ${peer.name}`;

  return (
    <>
      <FootnoteRow number={number} at={at} failed>
        <button
          type="button"
          className="footnote__send footnote__send--failed"
          onClick={() => setOpen(true)}
        >
          <span className="footnote__mark" aria-hidden="true">
            ⊘
          </span>
          {label}
          <span className="footnote__gist">{why(reason)}</span>
        </button>
      </FootnoteRow>

      {open && (
        <MessageModal
          title={label}
          peer={peer}
          at={at}
          body={body}
          refusal={reason}
          onClose={() => setOpen(false)}
        />
      )}
    </>
  );
}

export interface ModalProps {
  title: string;
  peer: WirePeer;
  counterpart?: WirePeer;
  hop?: number;
  at: number;
  body: string;
  refusal: string | null;
  onClose: () => void;
}

export function MessageModal({
  title,
  peer,
  counterpart,
  hop,
  at,
  body,
  refusal,
  onClose,
}: ModalProps) {
  useDismiss(onClose);

  return (
    <div className="scrim">
      <button type="button" className="scrim__close" aria-label="Close dialog" onClick={onClose} />
      <div className="dialog" role="dialog" aria-modal="true" aria-label={title}>
        <div className="wire-modal__head">
          <AgentAvatar avatar={peer.avatar} color={peer.color} size="sm" seed={peer.id} />
          {counterpart && (
            <>
              <span className="wire__arrow" aria-hidden="true">
                →
              </span>
              <AgentAvatar
                avatar={counterpart.avatar}
                color={counterpart.color}
                size="sm"
                seed={counterpart.id}
              />
            </>
          )}
          <h2 className="dialog__title" style={{ margin: 0 }}>
            {title}
          </h2>
          <span className="hint" style={{ marginLeft: "auto" }}>
            {hop !== undefined && `hop ${hop} · `}
            {clockTime(at)}
          </span>
        </div>

        {refusal && (
          <div className="banner" style={{ margin: "0 0 0.9rem" }}>
            <span>{refusal}</span>
          </div>
        )}

        <div className="wire-modal__body">
          {body ? <Markdown>{body}</Markdown> : <p className="hint">No content.</p>}
        </div>

        <div style={{ display: "flex", justifyContent: "flex-end", marginTop: "1rem" }}>
          <button type="button" className="btn" onClick={onClose}>
            Close
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * A peer composing a message to the agent whose channel this is.
 *
 * Deliberately textless. The finished message collapses into a burst row, so
 * streaming its text into a bubble first means the operator watches a wall of
 * text appear and then vanish, which is worse than never showing it.
 *
 * It is the chip that row will draw, with dots, and the words come from the
 * same function so they cannot drift apart. Written in its own shape instead
 * (an arrow, a sentence naming both ends) it moved, changed wording and grew an
 * avatar the instant the message landed, which read as two separate events
 * rather than one arriving.
 *
 * Only the sender is named because only the sender is news: agent-to-agent
 * traffic is filed under the recipient, so a live peer stream in this channel
 * is always addressed to the agent whose channel the operator already has open.
 */
export function WritingRow({ peer }: { peer: WirePeer }) {
  return (
    <div className="footnote" data-writing="true">
      <div className="footnote__margin" />
      <div className="footnote__body">
        <span className="footnote__line">
          <span className="footnote__send">
            {summaryLabel({ peer, agentId: null, sent: 0, received: 1, gist: null })}
          </span>
          <span className="footnote__dots" aria-hidden="true">
            <i />
            <i />
            <i />
          </span>
        </span>
      </div>
    </div>
  );
}

/**
 * A routine coming due, as one line.
 *
 * This used to be a chat bubble from "Guaca" carrying the whole instruction,
 * which is several sentences by design: a routine is written to be acted on
 * with no other context. An operator reading their own conversation with an
 * agent was shown the system prompting it, in the shape of somebody talking to
 * them, and the reflex is to read it as a message meant for you.
 *
 * So it is a chip. What it delivered is not lost: the click opens the routine
 * in the panel, where the instruction is the thing you came to read, and the
 * agent's answer is the next bubble either way.
 */
export function RoutineRow({
  title,
  what,
  payload,
  at,
  onOpen,
}: {
  title: string;
  what: string;
  /** What the event arrived with, when one did. Shown on hover, not drawn. */
  payload: string | null;
  at: number;
  onOpen: () => void;
}) {
  return (
    <div className="wire wire--routine">
      <button
        type="button"
        className="wire__chip wire__chip--open"
        title={
          payload
            ? `Open this routine. It asked: ${what}\n\nThe event arrived with:\n${payload}`
            : `Open this routine. It asked: ${what}`
        }
        onClick={onOpen}
      >
        <span className="routine__mark" aria-hidden="true" />
        <span className="wire__label">{title}</span>
        <span className="wire__why">routine ran</span>
        <span className="wire__meta">{clockTime(at)}</span>
      </button>
    </div>
  );
}

/**
 * What an agent wrote after it had already answered.
 *
 * A turn that replies to its peer through `send_message` and then keeps typing
 * has produced text with no recipient: the peer has been answered, and the
 * operator was never in that conversation. The runtime files it here rather
 * than delivering it, so this is deliberately not a bubble. A bubble in this
 * channel means the operator was addressed, and seven agents each posting one
 * to say they had replied to somebody else is exactly what that used to look
 * like.
 *
 * In the trail's column rather than the transcript's, because it is the same
 * kind of thing as the tool calls above it: what the turn did, not what it said
 * to anyone.
 */
export function AsideRow({ text }: { text: string }) {
  return (
    <div className="aside">
      <Markdown>{text}</Markdown>
    </div>
  );
}

/**
 * A centered system line: guard stops, upstream failures, lifecycle notes.
 *
 * `onRetry` is offered only where trying again could plausibly work. The
 * runtime has already retried a failed call several times by the time one of
 * these is written, so this is the operator's attempt, not the first one: a
 * button on a guard stop would only spend the same budget to hit the same
 * limit.
 */
export function NoticeRow({
  kind,
  text,
  onRetry,
}: {
  kind: string;
  text: string;
  onRetry?: () => void;
}) {
  const [tried, setTried] = useState(false);

  return (
    <div className="wire wire--notice">
      <span className={kind === "upstreamError" ? "chip chip--error" : "chip chip--guard"}>
        <span aria-hidden="true">{kind === "upstreamError" ? "!" : "◆"}</span>
        <span style={{ whiteSpace: "normal" }}>{text}</span>
        {onRetry && (
          <button
            type="button"
            className="chip__action"
            disabled={tried}
            onClick={() => {
              setTried(true);
              onRetry();
            }}
          >
            {tried ? "Sent again" : "Try again"}
          </button>
        )}
      </span>
    </div>
  );
}
