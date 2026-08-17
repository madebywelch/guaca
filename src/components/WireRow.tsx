import { useEffect, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
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
 * So a burst of peer traffic becomes a single centred line naming who was
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

/**
 * A burst of peer traffic, one chip per peer.
 *
 * Per peer rather than "5 messages with 2 agents", for the same reason a
 * fan-out draws one row per recipient: a count that does not name anyone
 * hides the thing the operator opened the channel to find out. It also means
 * every chip has exactly one thread behind it, so a click never has to ask
 * which conversation was meant.
 */
export function PeerBurstRow({
  peers,
  onOpen,
}: {
  peers: PeerSummary[];
  onOpen: (peer: AgentId) => void;
}) {
  return (
    <div className="wire wire--burst">
      {peers.map((summary) => {
        const { agentId } = summary;
        const face = (
          <>
            <AgentAvatar
              avatar={summary.peer.avatar}
              color={summary.peer.color}
              size="xs"
              seed={summary.peer.id}
            />
            <span className="wire__label">{summaryLabel(summary)}</span>
          </>
        );
        const style = { "--accent": summary.peer.color } as React.CSSProperties;

        // An unresolved name has no thread to open. It still gets a chip: an
        // agent that wrote to a name nobody can find is worth seeing.
        return agentId ? (
          <button
            key={summary.peer.id}
            type="button"
            className="wire__chip wire__chip--open"
            style={style}
            title={`Read the conversation with ${summary.peer.name}`}
            onClick={() => onOpen(agentId)}
          >
            {face}
          </button>
        ) : (
          <span key={summary.peer.id} className="wire__chip" style={style}>
            {face}
          </span>
        );
      })}
    </div>
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
  body,
  reason,
}: {
  peer: WirePeer;
  at: number;
  body: string;
  reason: string;
}) {
  const [open, setOpen] = useState(false);
  const label = `Not delivered to ${peer.name}`;

  return (
    <>
      <div className="wire" data-refused="true">
        <button
          type="button"
          className="wire__chip"
          onClick={() => setOpen(true)}
          style={{ "--accent": peer.color } as React.CSSProperties}
        >
          <span className="wire__arrow" aria-hidden="true">
            ⊘
          </span>
          <span className="wire__label">{label}</span>
          <span className="wire__why">{why(reason)}</span>
          <span className="wire__meta">{clockTime(at)}</span>
        </button>
      </div>

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
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

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
 * An agent composing a message to a peer.
 *
 * Deliberately textless. The finished message collapses into a burst row, so
 * streaming its text into a bubble first means the operator watches a wall of
 * text appear and then vanish, which is worse than never showing it.
 */
export function WritingRow({ from, to }: { from: WirePeer; to: WirePeer }) {
  return (
    <div className="wire wire--writing">
      <span className="wire__chip" style={{ "--accent": from.color } as React.CSSProperties}>
        <span className="wire__arrow" aria-hidden="true">
          ⇢
        </span>
        <span className="wire__label">
          {from.name} is writing to {to.name}
        </span>
        <span className="wire__dots" aria-hidden="true">
          <i />
          <i />
          <i />
        </span>
      </span>
    </div>
  );
}

/**
 * A centred system line: guard stops, upstream failures, lifecycle notes.
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
