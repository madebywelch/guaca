import { createContext, useCallback, useContext, useEffect, useRef, useState } from "react";
import { useDismiss } from "../lib/dismiss";
import { artifactUrl } from "../lib/files";
import { api } from "../lib/ipc";
import { type AgentId, errorMessage } from "../lib/types";
import { Icon } from "./Icon";

export const Answering = createContext<{ id: AgentId; name: string } | null>(null);

export function HtmlArtifact({
  html,
  title,
  data,
  onSend,
  fill = false,
}: {
  html: string;
  title: string;
  fill?: boolean;
  data?: unknown;
  onSend?: (json: string) => void;
}) {
  const frame = useRef<HTMLIFrameElement>(null);
  const openButton = useRef<HTMLButtonElement>(null);
  const returnFocus = useRef(false);
  const to = useContext(Answering);
  const sendTo = useRef(onSend);
  useEffect(() => {
    sendTo.current = onSend;
  }, [onSend]);
  const loaded = useRef(false);
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [height, setHeight] = useState(MIN_HEIGHT);
  const [full, setFull] = useState(false);
  const [ready, setReady] = useState<Ready | null>(null);
  const [sending, setSending] = useState(false);

  useEffect(() => {
    let live = true;
    setFailed(null);
    loaded.current = false;
    api
      .frameArtifact(html)
      .then((at) => live && setSrc(artifactUrl(at)))
      .catch((error) => live && setFailed(errorMessage(error)));
    return () => {
      live = false;
    };
  }, [html]);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      if (event.source !== frame.current?.contentWindow) return;
      const said: unknown = event.data;
      if (typeof said !== "object" || said === null) return;
      const {
        guaca,
        height: reported,
        value,
      } = said as {
        guaca?: unknown;
        height?: unknown;
        value?: unknown;
      };

      if (guaca === "artifact-height" && typeof reported === "number") {
        setHeight(Math.max(MIN_HEIGHT, Math.min(MAX_HEIGHT, Math.ceil(reported))));
        return;
      }

      if (guaca === "artifact-send" && typeof value === "string" && sendTo.current) {
        sendTo.current(value);
        return;
      }

      if ((guaca === "artifact-answer" || guaca === "artifact-send") && typeof value === "string") {
        setReady(
          value.length > MOST_ANSWER
            ? {
                kind: "refused",
                why: `This page tried to hand back ${value.length.toLocaleString()} characters, and ${MOST_ANSWER.toLocaleString()} is the most Guaca will carry. An answer is a choice, not a document.`,
              }
            : { kind: "value", json: value },
        );
      }
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  const post = useCallback(() => {
    if (data === undefined || !loaded.current) return;
    frame.current?.contentWindow?.postMessage({ guaca: "artifact-data", data }, "*");
  }, [data]);

  useEffect(() => {
    post();
  }, [post]);

  useDismiss(() => setFull(false), full);

  useEffect(() => {
    if (!full && returnFocus.current) {
      returnFocus.current = false;
      openButton.current?.focus();
    }
  }, [full]);

  if (failed) {
    return (
      <p className="hint">
        Could not show this page: {failed}. The source is above, and saving it to a file and opening
        it in a browser will always work.
      </p>
    );
  }

  const page = (
    <iframe
      ref={frame}
      className="artifact__frame"
      sandbox="allow-scripts"
      referrerPolicy="no-referrer"
      src={src ?? undefined}
      onLoad={() => {
        loaded.current = true;
        post();
      }}
      title={title}
      style={full || fill ? undefined : { height }}
    />
  );

  const answer = to && ready && (
    <Answer
      ready={ready}
      to={to}
      sending={sending}
      onDismiss={() => setReady(null)}
      onSend={async (json) => {
        setSending(true);
        try {
          await api.sendMessage(to.id, answerMessage(json));
          setReady({ kind: "sent" });
        } catch (error) {
          setReady({ kind: "refused", why: errorMessage(error) });
        } finally {
          setSending(false);
        }
      }}
    />
  );

  if (full) {
    return (
      <div className="scrim">
        <button
          type="button"
          className="scrim__close"
          aria-label="Close dialog"
          onClick={() => setFull(false)}
        />
        <div className="dialog dialog--file" role="dialog" aria-modal="true" aria-label={title}>
          <div className="file-view__head">
            <h2 className="dialog__title">{title}</h2>
            <div className="file-view__actions">
              <button
                type="button"
                className="file-view__close"
                aria-label="Close"
                title="Close"
                onClick={() => setFull(false)}
              >
                <Icon name="close" />
              </button>
            </div>
          </div>
          <div className="file-view__body artifact__full">
            <div className="artifact">{page}</div>
            {answer}
          </div>
        </div>
      </div>
    );
  }

  return (
    <>
      <div className={fill ? "artifact artifact--fill" : "artifact"}>
        {src ? page : <p className="hint">Opening…</p>}
        {!fill && (
          <button
            type="button"
            className="btn btn--quiet btn--compact artifact__open"
            ref={openButton}
            onClick={() => {
              returnFocus.current = true;
              setFull(true);
            }}
          >
            Open
          </button>
        )}
      </div>
      {answer}
    </>
  );
}

type Ready = { kind: "value"; json: string } | { kind: "refused"; why: string } | { kind: "sent" };

function Answer({
  ready,
  to,
  sending,
  onSend,
  onDismiss,
}: {
  ready: Ready;
  to: { id: AgentId; name: string };
  sending: boolean;
  onSend: (json: string) => void;
  onDismiss: () => void;
}) {
  if (ready.kind === "sent") {
    return (
      <div className="artifact__answer">
        <p className="artifact__answer-note">Sent to {to.name}.</p>
      </div>
    );
  }

  if (ready.kind === "refused") {
    return (
      <div className="artifact__answer">
        <p className="artifact__answer-note artifact__answer-note--fault">{ready.why}</p>
        <div className="artifact__answer-acts">
          <button type="button" className="btn btn--quiet btn--compact" onClick={onDismiss}>
            Dismiss
          </button>
        </div>
      </div>
    );
  }

  return (
    <div className="artifact__answer">
      <p className="artifact__answer-note">
        This page has an answer ready. Nothing goes to {to.name} until you send it.
      </p>
      <pre className="artifact__answer-value">{ready.json}</pre>
      <div className="artifact__answer-acts">
        <button
          type="button"
          className="btn btn--compact"
          disabled={sending}
          onClick={() => onSend(ready.json)}
        >
          {sending ? "Sending…" : `Send to ${to.name}`}
        </button>
        <button type="button" className="btn btn--quiet btn--compact" onClick={onDismiss}>
          Dismiss
        </button>
      </div>
    </div>
  );
}

export function answerMessage(json: string): string {
  const longest = Math.max(0, ...[...json.matchAll(/`+/g)].map((run) => run[0].length));
  const rail = "`".repeat(Math.max(3, longest + 1));
  return `From the page you drew:\n\n${rail}json\n${json}\n${rail}`;
}

const MOST_ANSWER = 4000;

const MIN_HEIGHT = 120;
const MAX_HEIGHT = 640;
