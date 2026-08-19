import { useEffect, useRef, useState } from "react";

import {
  fileUrl,
  localCopy,
  PREVIEW_BYTES,
  previewKind,
  readableSize,
  readFileText,
  SNIPPET_BYTES,
} from "../lib/files";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { type Attachment, errorMessage } from "../lib/types";

/**
 * A file on a message.
 *
 * Shown rather than named, because a document that arrives as a filename is a
 * document the operator has to open something else to read. A picture is drawn,
 * a PDF gets its first page, a text file gets its first lines, and anything
 * this cannot draw says so and offers the one thing that always works. All four
 * open a full view on a click.
 *
 * The bytes come over `guacfile:` rather than IPC: see `lib/files.ts`.
 */
export function FileCard({ file }: { file: Attachment }) {
  const [open, setOpen] = useState(false);
  const kind = previewKind(file.mime);

  return (
    <>
      <div className="file" data-kind={kind}>
        {kind !== "none" && (
          <button
            type="button"
            className="file__view"
            aria-label={`Open ${file.name}`}
            onClick={() => setOpen(true)}
          >
            <Thumbnail file={file} />
          </button>
        )}
        <div className="file__foot">
          <button type="button" className="file__name" onClick={() => setOpen(true)}>
            {file.name}
          </button>
          <span className="file__size">{readableSize(file.bytes)}</span>
          <SaveButton file={file} />
        </div>
      </div>
      {open && <FilePreview file={file} onClose={() => setOpen(false)} />}
    </>
  );
}

/**
 * As much of a file as belongs in a transcript.
 *
 * Bounded in height on purpose: a transcript is a conversation, and a document
 * that takes the whole pane has stopped being a message and become a reader.
 * The full view is one click away and is where the reading happens.
 */
function Thumbnail({ file }: { file: Attachment }) {
  switch (previewKind(file.mime)) {
    case "image":
      // Lazily, because a channel opens with three hundred messages in it and
      // the operator is looking at the last few.
      return <img className="file__image" src={fileUrl(file)} alt={file.name} loading="lazy" />;
    case "pdf":
      return <Page file={file} />;
    case "text":
      return <Snippet file={file} limit={SNIPPET_BYTES} />;
    default:
      return null;
  }
}

/**
 * The first page of a document, drawn by the webview's own viewer.
 *
 * A frame, and only once it is nearly on screen. Each one is a renderer of its
 * own holding a copy of the document, so mounting one per file the moment a
 * channel opens spends both on pages nobody has scrolled to. It takes no
 * clicks: the button behind it opens the full view, and a frame that swallowed
 * the click would leave a document whose one obvious way in does nothing.
 */
function Page({ file }: { file: Attachment }) {
  const [frame, near] = useNearViewport<HTMLDivElement>();
  const copy = useLocalCopy(file, near);

  return (
    <div className="file__page" ref={frame}>
      {copy.url && (
        <iframe className="file__frame" src={copy.url} title={file.name} tabIndex={-1} />
      )}
      {copy.failed && <p className="hint">This document could not be read.</p>}
    </div>
  );
}

/**
 * A copy of a whole file the frame can be pointed at, while it is wanted.
 *
 * Revoked on the way out, because this is the one place a file's bytes sit in
 * the renderer and a transcript scrolls past a lot of documents. `when` is what
 * keeps it to the ones on screen.
 */
function useLocalCopy(file: Attachment, when: boolean) {
  const [url, setUrl] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!when) return;
    let live = true;
    let made: string | null = null;
    localCopy(file)
      .then((copy) => {
        // A copy nobody is waiting for any more is revoked here rather than in
        // the cleanup, which has already run and seen no url to release.
        if (!live) return URL.revokeObjectURL(copy);
        made = copy;
        setUrl(copy);
      })
      .catch(() => live && setFailed(true));
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
      setUrl(null);
    };
  }, [file, when]);

  return { url, failed };
}

/**
 * The first lines of a text file, as text.
 *
 * `sayTrimmed` where the operator would otherwise believe they have read the
 * whole thing. Under a message the cut is plain from the shape of it; in the
 * full view it is not, and a preview that quietly stops is a lie.
 */
function Snippet({
  file,
  limit,
  sayTrimmed,
}: {
  file: Attachment;
  limit: number;
  sayTrimmed?: boolean;
}) {
  const [read, setRead] = useState<{ text: string; trimmed: boolean } | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let live = true;
    readFileText(file, limit)
      .then((got) => live && setRead(got))
      .catch(() => live && setFailed(true));
    return () => {
      live = false;
    };
  }, [file, limit]);

  if (failed) return <p className="hint">This file could not be read.</p>;
  return (
    <>
      <pre className="file__text">{read?.text ?? ""}</pre>
      {sayTrimmed && read?.trimmed && (
        <p className="hint">The first {readableSize(limit)}. Save a copy to read the rest.</p>
      )}
    </>
  );
}

/**
 * The file, as big as the window will allow.
 *
 * The same four shapes as the strip with the bounds taken off. A file with no
 * preview lands here too: the operator clicked something, and an explanation
 * with a way forward beats nothing happening.
 */
export function FilePreview({ file, onClose }: { file: Attachment; onClose: () => void }) {
  const kind = previewKind(file.mime);

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
      <div className="dialog dialog--file" role="dialog" aria-modal="true" aria-label={file.name}>
        <div className="file-view__head">
          <h2 className="dialog__title" style={{ margin: 0 }}>
            {file.name}
          </h2>
          <span className="hint">{readableSize(file.bytes)}</span>
          <div className="file-view__actions">
            <SaveButton file={file} />
            <button type="button" className="btn btn--ghost" onClick={onClose}>
              Close
            </button>
          </div>
        </div>

        <div className="file-view__body" data-kind={kind}>
          {kind === "image" ? (
            <img className="file-view__image" src={fileUrl(file)} alt={file.name} />
          ) : kind === "pdf" ? (
            <Document file={file} />
          ) : kind === "text" ? (
            <Snippet file={file} limit={PREVIEW_BYTES} sayTrimmed />
          ) : (
            <p className="hint">
              Nothing here can show this kind of file. Save a copy and open it with something that
              can.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

/** The whole document, in the full view. */
function Document({ file }: { file: Attachment }) {
  const copy = useLocalCopy(file, true);

  if (copy.failed) return <p className="hint">This document could not be read.</p>;
  return copy.url ? (
    <iframe className="file-view__frame" src={copy.url} title={file.name} />
  ) : (
    <p className="hint">Opening…</p>
  );
}

/**
 * Puts a copy where a person can get at it.
 *
 * The path comes back and is said out loud: a file saved somewhere the operator
 * has to go looking for has not really been saved. Imperative on the store, as
 * the retry button is, because this is one button on a component that is
 * rendered once per file in a transcript and holds no other state.
 */
function SaveButton({ file }: { file: Attachment }) {
  const [saving, setSaving] = useState(false);

  return (
    <button
      type="button"
      className="btn btn--ghost btn--small"
      disabled={saving}
      onClick={() => {
        setSaving(true);
        void api
          .saveFile(file.digest, file.name)
          .then((path) => useStore.getState().setBanner({ tone: "ok", text: `Saved to ${path}` }))
          .catch((error) =>
            useStore.getState().setBanner({ tone: "error", text: errorMessage(error) }),
          )
          .finally(() => setSaving(false));
      }}
    >
      {saving ? "Saving…" : "Save"}
    </button>
  );
}

/**
 * Whether a node has come near the window yet.
 *
 * The margin is generous because the point is to have mounted before the
 * operator arrives rather than as they arrive: a frame that starts loading once
 * it is already on screen is a grey rectangle they watch fill in.
 */
function useNearViewport<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [near, setNear] = useState(false);

  useEffect(() => {
    const node = ref.current;
    if (!node || near) return;
    const watching = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) setNear(true);
      },
      { rootMargin: "400px" },
    );
    watching.observe(node);
    return () => watching.disconnect();
  }, [near]);

  return [ref, near] as const;
}
