import { useEffect, useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import type { AgentCard } from "../lib/types";
import { Markdown } from "./Markdown";

/**
 * Agent-to-agent traffic, collapsed.
 *
 * A channel should read as a conversation with one agent. When every message
 * an agent exchanged with its peers is rendered as a full chat bubble, the
 * operator's own conversation is buried under machine chatter they were never
 * meant to read line by line.
 *
 * So peer traffic becomes a single centred line saying who spoke to whom, and
 * the content lives one click away. Full bubbles are reserved for the two
 * things actually addressed to the operator: what they said, and what an agent
 * said back.
 */

export interface WirePeer {
  name: string;
  color: string;
  avatar: string;
  id: string;
}

interface Props {
  /** Which way the message went, from this channel's point of view. */
  direction: "sent" | "received" | "between";
  peer: WirePeer;
  /** Only used for the `between` form, in the activity feed. */
  counterpart?: WirePeer;
  hop?: number;
  at: number;
  body: string;
  /** Shown instead of the body when nothing was delivered. */
  refusal?: string | null;
}

function clockTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function WireRow({ direction, peer, counterpart, hop, at, body, refusal }: Props) {
  const [open, setOpen] = useState(false);

  const label =
    direction === "sent"
      ? `Sent to ${peer.name}`
      : direction === "received"
        ? `Received from ${peer.name}`
        : `${peer.name} → ${counterpart?.name ?? "?"}`;

  const arrow = direction === "received" ? "⇠" : "⇢";

  return (
    <>
      <div className="wire" data-refused={refusal ? "true" : undefined}>
        <button
          type="button"
          className="wire__chip"
          onClick={() => setOpen(true)}
          style={{ "--accent": peer.color } as React.CSSProperties}
        >
          <span className="wire__arrow" aria-hidden="true">
            {refusal ? "⊘" : arrow}
          </span>
          <span className="wire__label">{refusal ? `Not delivered to ${peer.name}` : label}</span>
          {hop !== undefined && <span className="wire__meta">hop {hop}</span>}
          <span className="wire__meta">{clockTime(at)}</span>
        </button>
      </div>

      {open && (
        <MessageModal
          title={label}
          peer={peer}
          counterpart={counterpart}
          hop={hop}
          at={at}
          body={body}
          refusal={refusal ?? null}
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
 * Deliberately textless. The finished message collapses to a wire row, so
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

/** A centred system line: guard stops, upstream failures, lifecycle notes. */
export function NoticeRow({ kind, text }: { kind: string; text: string }) {
  return (
    <div className="wire wire--notice">
      <span className={kind === "upstreamError" ? "chip chip--error" : "chip chip--guard"}>
        <span aria-hidden="true">{kind === "upstreamError" ? "!" : "◆"}</span>
        <span style={{ whiteSpace: "normal" }}>{text}</span>
      </span>
    </div>
  );
}

/**
 * Resolves an agent to the shape the wire row needs.
 *
 * `fallbackName` matters: a tool call names its recipients, and a name that
 * never resolved is far more useful shown as written than as "Deleted agent".
 * An agent that wrote to "Nobody" should read as having written to Nobody.
 */
export function toPeer(
  card: AgentCard | undefined,
  fallbackId: string,
  fallbackName = "Deleted agent",
): WirePeer {
  return card
    ? { id: card.id, name: card.name, color: card.color, avatar: card.avatar }
    : { id: fallbackId, name: fallbackName, color: "#8aa0a6", avatar: "blank" };
}
