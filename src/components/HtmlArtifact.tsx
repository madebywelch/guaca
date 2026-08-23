import { useEffect, useRef, useState } from "react";

import { api } from "../lib/ipc";
import { errorMessage } from "../lib/types";

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
 */
export function HtmlArtifact({ html, title }: { html: string; title: string }) {
  const frame = useRef<HTMLIFrameElement>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [height, setHeight] = useState(MIN_HEIGHT);
  const [full, setFull] = useState(false);

  useEffect(() => {
    let live = true;
    setFailed(null);
    api
      .frameArtifact(html)
      .then((at) => live && setSrc(`http://127.0.0.1:${at.port}/${at.id}`))
      .catch((error) => live && setFailed(errorMessage(error)));
    return () => {
      live = false;
    };
  }, [html]);

  // A frame on another origin cannot be measured from outside, so the page is
  // asked. The message is trusted by the window it came from and by nothing
  // else: an opaque origin reports itself as "null", so checking the origin
  // would either reject every real message or accept every forged one.
  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      if (event.source !== frame.current?.contentWindow) return;
      const said: unknown = event.data;
      if (typeof said !== "object" || said === null) return;
      const { guaca, height: reported } = said as { guaca?: unknown; height?: unknown };
      if (guaca !== "artifact-height" || typeof reported !== "number") return;
      // Clamped rather than obeyed. A page that reports a height of a hundred
      // thousand has made itself the whole channel, and one that reports zero
      // has made itself invisible.
      setHeight(Math.max(MIN_HEIGHT, Math.min(MAX_HEIGHT, Math.ceil(reported))));
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
          </div>
        </div>
      </div>
    );
  }

  return (
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
  );
}

/** Tall enough that a page still drawing does not read as a broken frame. */
const MIN_HEIGHT = 120;
/** And short enough that one cannot make itself the whole channel. */
const MAX_HEIGHT = 640;
