import { createContext, useContext, useEffect, useRef, useState } from "react";

import { artifactUrl } from "../lib/files";
import { api } from "../lib/ipc";
import { type AgentId, errorMessage } from "../lib/types";

/**
 * Who a page drawn here may answer, if anybody.
 *
 * Null by default and provided by exactly one surface: the channel, where the
 * operator is one of the two participants and their next message has an
 * obvious recipient. A page behind a search hit, inside a pair's thread or in a
 * document preview has nobody, and a Send button in those places would be a
 * control that cannot say who it sends to. Same discipline as `Roster`: the
 * default is honest, so a surface that has not thought about it draws nothing.
 */
export const Answering = createContext<{ id: AgentId; name: string } | null>(null);

/**
 * A page an agent wrote, running.
 *
 * The frame points at a loopback origin of Guaca's own rather than carrying
 * the markup inline, because a frame given `srcdoc` inherits this document's
 * content policy and this document forbids script. A page framed that way
 * draws and then quietly does nothing, which is the worst thing this could
 * ship: it passes every test and shows the operator a blank rectangle.
 *
 * What the page may do once it is running is argued in `artifact.rs`, and none
 * of it is decided here. The short version is that it may compute and it may
 * draw, and it may not reach anything: no fetch, no remote image, no form. The
 * `sandbox` attribute below is the second half of the same lock and its two
 * values must never gain `allow-same-origin`, which would let the page take the
 * sandbox off and reload out of it.
 *
 * The one thing it may hand back is a value, and this component is the reason
 * that is not a hole in the paragraph above. `guaca.answer` posts; this draws
 * what was posted, in Guaca's chrome, under the frame, and waits. Nothing goes
 * anywhere until the operator presses a button that belongs to the app.
 */
export function HtmlArtifact({ html, title }: { html: string; title: string }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const to = useContext(Answering);
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [height, setHeight] = useState(MIN_HEIGHT);
  const [full, setFull] = useState(false);
  const [ready, setReady] = useState<Ready | null>(null);
  const [sending, setSending] = useState(false);

  useEffect(() => {
    let live = true;
    setFailed(null);
    api
      .frameArtifact(html)
      .then((at) => live && setSrc(artifactUrl(at)))
      .catch((error) => live && setFailed(errorMessage(error)));
    return () => {
      live = false;
    };
  }, [html]);

  // Both of the things a page can say. Each is trusted by the window it came
  // from and by nothing else: an opaque origin reports itself as "null", so
  // checking the origin would either reject every real message or accept every
  // forged one. Bound once, with no dependency on who is being answered: a page
  // in a surface with nobody to answer still reports its height, and the value
  // it posts is simply never drawn.
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
        // Clamped rather than obeyed. A page that reports a height of a hundred
        // thousand has made itself the whole channel, and one that reports zero
        // has made itself invisible.
        setHeight(Math.max(MIN_HEIGHT, Math.min(MAX_HEIGHT, Math.ceil(reported))));
        return;
      }

      if (guaca === "artifact-answer" && typeof value === "string") {
        // Replaces whatever was there rather than queuing. A page that answers
        // on every drag of a slider is doing the right thing, and what the
        // operator sends is what the page last said, which is what they are
        // looking at.
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

  useEffect(() => {
    if (!full) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setFull(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [full]);

  if (failed) {
    return (
      <p className="hint">
        Could not show this page: {failed}. The source is above, and saving it to a file and opening
        it in a browser will always work.
      </p>
    );
  }

  // One frame, in one of two places. The same element written into both would
  // be two of them: React draws it once per position in the tree, so opening
  // the full view would start a second renderer loading the same page, and the
  // ref would end up on whichever mounted last.
  const page = (
    <iframe
      ref={frame}
      className="artifact__frame"
      // Scripts, and deliberately nothing else. Never `allow-same-origin`
      // beside `allow-scripts`: together they let the page remove its own
      // sandbox and reload without one.
      sandbox="allow-scripts"
      referrerPolicy="no-referrer"
      src={src ?? undefined}
      title={title}
      style={full ? undefined : { height }}
    />
  );

  // Drawn wherever the frame is. A page answered in the full view and shown the
  // strip only on the card behind it is an operator pressing a button and
  // watching nothing happen.
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
                ×
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
      <div className="artifact">
        {src ? page : <p className="hint">Opening…</p>}
        {/* A transcript is a conversation, so a page in one is bounded however
            tall it says it is. The full view is where the reading happens, the
            same way a document works. */}
        <button
          type="button"
          className="btn btn--ghost btn--small artifact__open"
          onClick={() => setFull(true)}
        >
          Open
        </button>
      </div>
      {answer}
    </>
  );
}

/** What a page has handed back, and what became of it. */
type Ready = { kind: "value"; json: string } | { kind: "refused"; why: string } | { kind: "sent" };

/**
 * The strip under a page that has answered.
 *
 * Outside the frame, in the app's own chrome, and that is the entire point
 * rather than a layout choice. The page draws the controls the operator works;
 * Guaca draws the one that spends a turn. A page cannot press this, cannot
 * style it, and cannot reach the document it is drawn in.
 *
 * The value is shown before it goes and shown as text. It is a model's own JSON
 * about to travel to an agent under the operator's name, which is the shape
 * this app treats most carefully everywhere else it occurs.
 */
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
          <button type="button" className="btn btn--ghost btn--small" onClick={onDismiss}>
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
          className="btn btn--small"
          disabled={sending}
          onClick={() => onSend(ready.json)}
        >
          {sending ? "Sending…" : `Send to ${to.name}`}
        </button>
        <button type="button" className="btn btn--ghost btn--small" onClick={onDismiss}>
          Dismiss
        </button>
      </div>
    </div>
  );
}

/**
 * The message the agent actually receives.
 *
 * Guaca's sentence around the page's JSON, and the split is deliberate: a page
 * hands back a value and never a sentence, so nothing it wrote can arrive as an
 * instruction in the operator's voice. The operator read the value in the strip
 * and pressed the button, so this is their message and it is `Trust::Operator`
 * like any other; what they are vouching for is a value they could see.
 *
 * Fenced with a run one longer than the longest run in the value, which is
 * CommonMark's own rule. A choice like `{"snippet": "```"}` is a perfectly
 * ordinary thing for a page about code to hand back, and a fixed three would
 * end the block in the middle of it.
 */
export function answerMessage(json: string): string {
  const longest = Math.max(0, ...[...json.matchAll(/`+/g)].map((run) => run[0].length));
  const rail = "`".repeat(Math.max(3, longest + 1));
  return `From the page you drew:\n\n${rail}json\n${json}\n${rail}`;
}

/**
 * The most JSON a page may hand back.
 *
 * An answer is what the operator chose, not what the page holds: a plan, a
 * range, the six rows they ticked. Comfortably more than a long form's worth of
 * values, and far less than a message anybody would read before sending, which
 * is the thing the strip asks them to do.
 */
const MOST_ANSWER = 4000;

/** Tall enough that a page still drawing does not read as a broken frame. */
const MIN_HEIGHT = 120;
/** And short enough that one cannot make itself the whole channel. */
const MAX_HEIGHT = 640;
