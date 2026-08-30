import { useEffect, useRef, useState } from "react";

import {
  fileUrl,
  kindLabel,
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
import { Markdown } from "./Markdown";

/**
 * A file on a message.
 *
 * Shown rather than named, because a document that arrives as a filename is a
 * document the operator has to open something else to read. A picture is drawn,
 * a PDF gets its first page, a markdown file is drawn as the document it is, a
 * text file gets its first lines, and anything this cannot draw says so and
 * offers the one thing that always works. All five open a full view on a click.
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
          <button
            type="button"
            className="file__name"
            title={file.name}
            onClick={() => setOpen(true)}
          >
            {file.name}
          </button>
          {/* The type only where nothing else says it. A picture, a page and a
              log announce themselves; a zip is a name and a size and no other
              fact, which is the row that leaves an operator wondering what the
              app thinks it is holding. */}
          <span className="file__meta">
            {kind === "none" && `${kindLabel(file.mime)} · `}
            {readableSize(file.bytes)}
          </span>
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
    case "markdown":
      return <Snippet file={file} limit={SNIPPET_BYTES} render="markdown" />;
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
      {copy.failed && <Unreadable file={file} why={copy.failed} onRetry={copy.again} />}
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
  const [failed, setFailed] = useState<string | null>(null);
  const [attempt, again] = useAttempt();

  useEffect(() => {
    if (!when) return;
    let live = true;
    let made: string | null = null;
    setFailed(null);
    localCopy(file)
      .then((copy) => {
        // A copy nobody is waiting for any more is revoked here rather than in
        // the cleanup, which has already run and seen no url to release.
        if (!live) return URL.revokeObjectURL(copy);
        made = copy;
        setUrl(copy);
      })
      .catch((error) => live && setFailed(errorMessage(error)));
    return () => {
      live = false;
      if (made) URL.revokeObjectURL(made);
      setUrl(null);
    };
  }, [file, when, attempt]);

  return { url, failed, again };
}

/**
 * A counter that makes an effect run again.
 *
 * Reading a file is the one thing in a transcript that can fail for a reason
 * that has since stopped being true: the store answers 503 while it is opening,
 * and a preview that mounted in that window stays broken for the life of the
 * channel with no way back other than closing it. So every failure offers
 * another go.
 */
function useAttempt() {
  const [attempt, setAttempt] = useState(0);
  return [attempt, () => setAttempt((made) => made + 1)] as const;
}

/**
 * A file the app has, and could not read.
 *
 * Says which of the reasons it was and offers the two ways forward, because
 * every error the operator can hit in this app says what happened and what to
 * do about it, and this one used to say neither. "This file could not be read"
 * is the same sentence whether the store was still opening, the bytes are
 * missing, or something else entirely went wrong, and those want different
 * things done about them.
 */
function Unreadable({
  file,
  why,
  onRetry,
}: {
  file: Attachment;
  why: string;
  onRetry: () => void;
}) {
  return (
    <div className="file__failed">
      <p className="hint">Could not read this file: {why}.</p>
      <div className="file__failed-actions">
        <button type="button" className="btn btn--ghost btn--small" onClick={onRetry}>
          Try again
        </button>
        <SaveButton file={file} />
      </div>
    </div>
  );
}

/**
 * The first lines of a text file.
 *
 * `render` decides whether they are drawn as their own source or as the
 * document they describe. Markdown is drawn: it is what the agents write in,
 * and a brief shown as monospace `##` is one the operator reads around rather
 * than through. Everything else is source, because a log is not prose and
 * markdown rules applied to one would eat its punctuation.
 *
 * The same trust as a message body, and the same renderer: no raw HTML, and
 * links leave through the operator's own browser. A file from an agent's
 * machine is no more trustworthy than the message that carried it.
 *
 * `sayTrimmed` where the operator would otherwise believe they have read the
 * whole thing. Under a message the cut is plain from the shape of it; in the
 * full view it is not, and a preview that quietly stops is a lie.
 */
function Snippet({
  file,
  limit,
  render = "source",
  sayTrimmed,
}: {
  file: Attachment;
  limit: number;
  render?: "source" | "markdown";
  sayTrimmed?: boolean;
}) {
  const [read, setRead] = useState<{ text: string; trimmed: boolean } | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const [attempt, again] = useAttempt();

  useEffect(() => {
    let live = true;
    setFailed(null);
    readFileText(file, limit)
      .then((got) => live && setRead(got))
      .catch((error) => live && setFailed(errorMessage(error)));
    return () => {
      live = false;
    };
  }, [file, limit, attempt]);

  if (failed) return <Unreadable file={file} why={failed} onRetry={again} />;
  return (
    <>
      {render === "markdown" ? (
        <div className="file__doc">
          <Markdown>{read?.text ?? ""}</Markdown>
        </div>
      ) : (
        <pre className="file__text">{read?.text ?? ""}</pre>
      )}
      {sayTrimmed && read?.trimmed && (
        <p className="hint">The first {readableSize(limit)}. Save a copy to read the rest.</p>
      )}
    </>
  );
}

/**
 * The file, as big as the window will allow.
 *
 * The same five shapes as the strip with the bounds taken off. A file with no
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
          {/* The name truncates and the whole of it is a hover away. What must
              never fold is the line under it: `48 KB` broken across two lines
              is the dialog telling the operator its own layout gave up. */}
          <h2 className="dialog__title" title={file.name}>
            {file.name}
          </h2>
          <p className="file-view__what">
            {kindLabel(file.mime)} · {readableSize(file.bytes)}
          </p>
          <div className="file-view__actions">
            <SaveButton file={file} />
            {/* Its own corner rather than a second button beside Save. Side by
                side they read as a pair of answers to a question nobody asked,
                which is how "Save" came to look like it might change the file
                rather than copy it out. */}
            <button
              type="button"
              className="file-view__close"
              aria-label="Close"
              title="Close"
              onClick={onClose}
            >
              ×
            </button>
          </div>
        </div>

        <div className="file-view__body" data-kind={kind}>
          {kind === "image" ? (
            <img className="file-view__image" src={fileUrl(file)} alt={file.name} />
          ) : kind === "pdf" ? (
            <Document file={file} />
          ) : kind === "markdown" ? (
            <Snippet file={file} limit={PREVIEW_BYTES} render="markdown" sayTrimmed />
          ) : kind === "text" ? (
            <Snippet file={file} limit={PREVIEW_BYTES} sayTrimmed />
          ) : (
            <p className="hint">
              Guaca cannot show a {kindLabel(file.mime).toLowerCase()}. Save a copy and open it with
              something that can.
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

  if (copy.failed) return <Unreadable file={file} why={copy.failed} onRetry={copy.again} />;
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
 *
 * It says what it does. "Save" beside a document is the word every editor uses
 * for writing changes back, so on a file the operator did not write and cannot
 * edit it reads as a button whose effect is anybody's guess. This one copies
 * the file out to the downloads folder and never touches the original, and
 * three words are cheaper than the guess.
 */
function SaveButton({ file }: { file: Attachment }) {
  const [saving, setSaving] = useState(false);
  const localFiles = useStore((s) => s.capabilities.localFiles);

  // On a server there is no downloads folder on the box that means anything
  // to the operator, and the browser has its own. The same bytes, on the route
  // every preview already reads from, handed to the browser as a download.
  if (!localFiles) {
    return (
      <a
        className="btn btn--ghost btn--small"
        href={fileUrl(file)}
        download={file.name}
        title={`Download ${file.name}`}
      >
        Download
      </a>
    );
  }

  return (
    <button
      type="button"
      className="btn btn--ghost btn--small"
      title={`Copy ${file.name} to your downloads folder`}
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
      {saving ? "Saving…" : "Save a copy"}
    </button>
  );
}

/**
 * Whether a node has come near the window yet.
 *
 * The margin is generous because the point is to have mounted before the
 * operator arrives rather than as they arrive: a frame that starts loading once
 * it is already on screen is a gray rectangle they watch fill in.
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
