import { useEffect, useRef, useState } from "react";

import { useDismiss } from "../lib/dismiss";
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
import { desktop } from "../lib/transport";
import { type Attachment, errorMessage } from "../lib/types";
import { Markdown } from "./Markdown";

export function FileCard({ file }: { file: Attachment }) {
  const [open, setOpen] = useState(false);
  const kind = previewKind(file.mime);

  return (
    <>
      <div className="file" data-kind={kind}>
        {kind !== "none" && (
          <div className="file__view" style={{ position: "relative" }}>
            <button
              type="button"
              aria-label={`Open ${file.name}`}
              onClick={() => setOpen(true)}
              style={{
                position: "absolute",
                inset: 0,
                width: "100%",
                background: "transparent",
                border: 0,
              }}
            />
            <div style={{ position: "relative", pointerEvents: "none" }}>
              <Thumbnail file={file} />
            </div>
          </div>
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

function Thumbnail({ file }: { file: Attachment }) {
  switch (previewKind(file.mime)) {
    case "image":
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

function useAttempt() {
  const [attempt, setAttempt] = useState(0);
  return [attempt, () => setAttempt((made) => made + 1)] as const;
}

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
    <div className="file__failed" style={{ pointerEvents: "auto" }}>
      <p className="hint">Could not read this file: {why}.</p>
      <div className="file__failed-actions">
        <button
          type="button"
          className="btn btn--ghost btn--small"
          onClick={(event) => {
            event.stopPropagation();
            onRetry();
          }}
        >
          Try again
        </button>
        <SaveButton file={file} />
      </div>
    </div>
  );
}

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

export function FilePreview({ file, onClose }: { file: Attachment; onClose: () => void }) {
  const kind = previewKind(file.mime);

  useDismiss(onClose);

  return (
    <div className="scrim">
      <button type="button" className="scrim__close" aria-label="Close dialog" onClick={onClose} />
      <div className="dialog dialog--file" role="dialog" aria-modal="true" aria-label={file.name}>
        <div className="file-view__head">
          <h2 className="dialog__title" title={file.name}>
            {file.name}
          </h2>
          <p className="file-view__what">
            {kindLabel(file.mime)} · {readableSize(file.bytes)}
          </p>
          <div className="file-view__actions">
            <SaveButton file={file} />
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

function Document({ file }: { file: Attachment }) {
  const copy = useLocalCopy(file, true);

  if (copy.failed) return <Unreadable file={file} why={copy.failed} onRetry={copy.again} />;
  return copy.url ? (
    <iframe className="file-view__frame" src={copy.url} title={file.name} />
  ) : (
    <p className="hint">Opening…</p>
  );
}

function SaveButton({ file }: { file: Attachment }) {
  const [saving, setSaving] = useState(false);
  if (!desktop) {
    return (
      <a
        className="btn btn--ghost btn--small"
        href={`${fileUrl(file)}&download=true`}
        download={file.name}
        target="_blank"
        rel="noopener noreferrer"
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
          .catch((error) => useStore.getState().showFailure(error))
          .finally(() => setSaving(false));
      }}
    >
      {saving ? "Saving…" : "Save a copy"}
    </button>
  );
}

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
