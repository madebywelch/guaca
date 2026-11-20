import { useId, useState } from "react";
import { AgentAvatar } from "../avatars/AgentAvatar";
import { useDismiss } from "../lib/dismiss";
import { foldLine, peerState, settled, spanOf, statusWord } from "../lib/handoff";
import { useStore } from "../lib/store";
import { elapsed, useNow } from "../lib/time";
import { type PeerSummary, summaryLabel, type WirePeer } from "../lib/transcript";
import type { AgentId, RunId } from "../lib/types";
import { Icon } from "./Icon";
import { Markdown } from "./Markdown";

export function why(reason: string): string {
  const body = reason.replace(/^Refused:\s*/i, "");
  const first = body.split(/(?<=\.)\s/)[0] ?? body;
  return first.replace(/\.$/, "");
}

function clockTime(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function FootnoteRow({ children, failed }: { children: React.ReactNode; failed?: boolean }) {
  return (
    <div className="footnote" data-failed={failed || undefined}>
      {children}
    </div>
  );
}

function PeerLine({
  peers,
  channel,
  rowKey,
  onOpen,
}: {
  peers: PeerSummary[];
  channel: string;
  rowKey: string;
  onOpen: (peer: AgentId) => void;
}) {
  return (
    <FootnoteRow>
      <span className="footnote__line">
        {peers.map((summary) => (
          <span className="footnote__item" key={summary.peer.id}>
            {summary.agentId ? (
              <button
                type="button"
                className="footnote__send"
                data-dismiss-id={JSON.stringify([channel, rowKey, summary.agentId])}
                title={`Read the conversation with ${summary.peer.name}`}
                onClick={() => summary.agentId && onOpen(summary.agentId)}
              >
                {summaryLabel(summary)}
              </button>
            ) : (
              <span className="footnote__send">{summaryLabel(summary)}</span>
            )}
            {summary.gist && <span className="footnote__gist">{summary.gist}</span>}
          </span>
        ))}
      </span>
    </FootnoteRow>
  );
}

export function HandoffRow({
  peers,
  runId,
  channel,
  rowKey,
  open,
  onToggle,
  onOpen,
}: {
  peers: PeerSummary[];
  runId: RunId;
  channel: string;
  rowKey: string;
  open: boolean;
  onToggle: () => void;
  onOpen: (peer: AgentId) => void;
}) {
  const live = useStore((state) => state.running.some((one) => one.runId === runId));

  if (peers.every((summary) => summary.sent === 0)) {
    return <PeerLine peers={peers} channel={channel} rowKey={rowKey} onOpen={onOpen} />;
  }

  if (live && !settled(peers)) {
    return (
      <div className="handoff">
        <HandoffCard
          peers={peers}
          runId={runId}
          channel={channel}
          rowKey={rowKey}
          live
          onOpen={onOpen}
        />
      </div>
    );
  }

  return (
    <HandoffRecord
      peers={peers}
      runId={runId}
      channel={channel}
      rowKey={rowKey}
      open={open}
      onToggle={onToggle}
      onOpen={onOpen}
    />
  );
}

export function HandoffRecord({
  peers,
  runId,
  channel,
  rowKey,
  open,
  onToggle,
  onOpen,
}: {
  peers: PeerSummary[];
  runId: RunId;
  channel: string;
  rowKey: string;
  open: boolean;
  onToggle: () => void;
  onOpen: (peer: AgentId) => void;
}) {
  const region = useId();
  const span = spanOf(peers);
  return (
    <div className="handoff">
      <button
        type="button"
        className="handoff__fold"
        aria-expanded={open}
        aria-controls={open ? region : undefined}
        onClick={onToggle}
      >
        <span className="handoff__faces" aria-hidden="true">
          {peers.map(({ peer }) => (
            <AgentAvatar
              key={peer.id}
              avatar={peer.avatar}
              color={peer.color}
              size="xs"
              seed={peer.id}
            />
          ))}
        </span>
        <span className="handoff__line">{foldLine(peers)}</span>
        <Icon name={open ? "chevron-down" : "chevron-right"} size={16} />
        {span !== null && <span className="handoff__took">{elapsed(span, 0)}</span>}
      </button>
      {open && (
        <div id={region}>
          <HandoffCard
            peers={peers}
            runId={runId}
            channel={channel}
            rowKey={rowKey}
            live={false}
            onOpen={onOpen}
          />
        </div>
      )}
    </div>
  );
}

function HandoffCard({
  peers,
  runId,
  channel,
  rowKey,
  live,
  onOpen,
}: {
  peers: PeerSummary[];
  runId: RunId;
  channel: string;
  rowKey: string;
  live: boolean;
  onOpen: (peer: AgentId) => void;
}) {
  const activity = useStore((state) => state.activity);
  const activeRun = useStore((state) => state.activeRun);
  const writingIds = useStore((state) =>
    Object.values(state.streams)
      .flatMap((buffer) => (buffer?.channelId === channel ? [buffer.agentId] : []))
      .join(" "),
  );
  const now = useNow(live ? 1000 : 60_000);
  const writing = new Set(writingIds.split(" "));
  const asked = peers.filter((summary) => summary.sent > 0).length;

  return (
    <section
      className="handoff__card"
      aria-label={
        asked > 0
          ? `Asked ${asked === 1 ? "1 colleague" : `${asked} colleagues`}`
          : "Heard from colleagues"
      }
    >
      <p className="handoff__title">
        {asked > 0 ? `Asked ${asked === 1 ? "1 colleague" : `${asked} colleagues`}` : "Heard from"}
      </p>
      <ul className="handoff__list">
        {peers.map((summary) => {
          const { peer, agentId } = summary;
          const state = peerState(summary, runId, { activity, activeRun, writing });
          return (
            <li key={peer.id} className="handoff__peer" data-state={state.kind}>
              <AgentAvatar
                avatar={peer.avatar}
                color={peer.color}
                size="sm"
                seed={peer.id}
                activity={
                  state.kind === "working" || state.kind === "writing"
                    ? { state: "thinking" }
                    : undefined
                }
              />
              <div className="handoff__text">
                <p className="handoff__who">
                  {agentId ? (
                    <button
                      type="button"
                      className="handoff__name"
                      data-dismiss-id={JSON.stringify([channel, rowKey, agentId])}
                      title={`Read the conversation with ${peer.name}`}
                      onClick={() => onOpen(agentId)}
                    >
                      {peer.name}
                    </button>
                  ) : (
                    <span className="handoff__name">{peer.name}</span>
                  )}
                  <span className="handoff__status">{statusWord(state, now)}</span>
                </p>
                {summary.asked && <p className="handoff__question">{summary.asked}</p>}
                {summary.answer && (
                  <div className="handoff__answer">
                    <Markdown>{summary.answer}</Markdown>
                  </div>
                )}
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

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
      <FootnoteRow failed>
        <button
          type="button"
          className="footnote__send footnote__send--failed"
          onClick={() => setOpen(true)}
        >
          <span className="footnote__mark" aria-hidden="true">
            <Icon name="blocked" size={16} />
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
                <Icon name="arrow-right" size={16} />
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
      <div>
        <span className="footnote__line">
          <span className="footnote__send">{summaryLabel({ peer, sent: 0, received: 1 })}</span>
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
        <Icon name={kind === "upstreamError" ? "alert" : "shield"} size={16} />
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
