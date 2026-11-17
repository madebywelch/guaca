import { useState } from "react";

import { AgentAvatar } from "../avatars/AgentAvatar";
import { useDismiss } from "../lib/dismiss";

import { type PeerSummary, summaryLabel, type WirePeer } from "../lib/transcript";
import type { AgentId } from "../lib/types";
import { Markdown } from "./Markdown";

export function why(reason: string): string {
  const body = reason.replace(/^Refused:\s*/i, "");
  const first = body.split(/(?<=\.)\s/)[0] ?? body;
  return first.replace(/\.$/, "");
}

function clockTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function PeerChip({ summary, onOpen }: { summary: PeerSummary; onOpen: (peer: AgentId) => void }) {
  const { peer, agentId } = summary;
  const label = summaryLabel(summary);

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

export function RoutineRow({
  title,
  what,
  payload,
  at,
  onOpen,
}: {
  title: string;
  what: string;
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

export function AsideRow({ text }: { text: string }) {
  return (
    <div className="aside">
      <Markdown>{text}</Markdown>
    </div>
  );
}

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
        <span style={{ whiteSpace: "pre-line" }}>{text}</span>
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
