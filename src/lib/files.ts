/**
 * How the webview gets at a file, and what it can show of one.
 *
 * The bytes never cross IPC: a transcript does, in bulk, which is the whole
 * reason a message carries a digest rather than a document. They come over
 * `guacfile:` instead, a scheme `app.rs` answers out of the file store, so a
 * picture is fetched once by the element drawing it and only while that element
 * is on screen.
 *
 * A hosted workspace answers the same bytes on a route instead, because a
 * custom scheme is a thing a webview registers and a browser has none. The one
 * difference is where the token goes: an `<img>` cannot carry a header, so it
 * goes in the query string, exactly as the event socket's does.
 */

import { convertFileSrc } from "@tauri-apps/api/core";

import { hosted, token, workspaceOrigin } from "./transport";
import type { Attachment } from "./types";

/** The scheme `app.rs` registers. */
const SCHEME = "guacfile";

/** How much of a text file the strip under a message reads. */
export const SNIPPET_BYTES = 2048;

/**
 * And how much the full view reads.
 *
 * A cap rather than the whole file, because "preview" is a promise about how
 * long this takes. A log nobody meant to attach is 20 MB, and the operator
 * asked to glance at it, not to load it.
 */
export const PREVIEW_BYTES = 256 * 1024;

/** What can be drawn of a file, decided by its type and nothing else. */
export type PreviewKind = "image" | "pdf" | "markdown" | "text" | "none";

/**
 * Which of the five a file is.
 *
 * By type, never by sniffing the bytes, which is the same rule the runtime
 * follows when it decides what to show a model. A file that is only mostly text
 * renders as a screenful of replacement characters, and the operator is better
 * served by a row saying what it is and a button that saves it.
 *
 * Markdown is its own kind because it is the format the agents actually write
 * in. A brief drawn as monospace source is a document the operator reads around
 * the syntax rather than through it, and the app already renders every message
 * body the same way: a file is the same prose that arrived without one.
 */
export function previewKind(mime: string): PreviewKind {
  if (mime.startsWith("image/")) return "image";
  if (mime === "application/pdf") return "pdf";
  if (mime === "text/markdown") return "markdown";
  if (mime.startsWith("text/") || mime === "application/json") return "text";
  return "none";
}

/**
 * What kind of file this is, in the words a person would use.
 *
 * Only ever drawn where nothing else says it: a picture, a page of a document
 * and the first lines of a log all announce themselves, and a label under one
 * of those is a caption on a photograph of a cat saying "cat". A zip and a
 * spreadsheet announce nothing at all, and a row carrying a name and a size and
 * no other fact is the row that makes an operator ask what the app even thinks
 * it is holding.
 *
 * From the type rather than the extension, because the type is what every
 * other decision about this file was made from and a label that disagreed with
 * the preview would be worse than no label.
 */
export function kindLabel(mime: string): string {
  if (mime === "application/pdf") return "PDF";
  if (mime === "application/zip") return "ZIP archive";
  // Ahead of the `text/` rule below, which would shout it.
  if (mime === "text/markdown") return "Markdown";
  if (mime.startsWith("image/")) return `${mime.slice("image/".length).toUpperCase()} image`;
  if (mime.startsWith("text/")) return mime.slice("text/".length).toUpperCase();
  if (mime === "application/json") return "JSON";
  if (mime.includes("wordprocessingml") || mime === "application/msword") return "Word document";
  if (mime.includes("spreadsheetml") || mime === "application/vnd.ms-excel") return "Spreadsheet";
  if (mime === "application/octet-stream") return "File";
  return mime;
}

/**
 * Where the bytes of a stored file are, as a URL.
 *
 * The name travels with the digest because the type is worked out from it on
 * the other side: the same function that typed the file when it arrived. It is
 * not what finds the file. Two agents sending the same document under different
 * names are one set of bytes at one address.
 */
export function fileUrl(file: Pick<Attachment, "digest" | "name">): string {
  if (hosted) {
    const target = `${encodeURIComponent(file.digest)}/${encodeURIComponent(file.name)}`;
    return `${workspaceOrigin()}/v1/file/${target}?token=${encodeURIComponent(token())}`;
  }
  // Asked for rather than built: Tauri spells its own scheme differently per
  // platform (`guacfile:` on macOS, `http://guacfile.localhost` on Windows,
  // which is why the app's content policy names both), and a hand-built URL is
  // right on the machine it was written on and draws nothing on the other.
  //
  // Imported statically rather than behind the host check, because this has to
  // be synchronous: it is called while a message renders. The function is pure
  // string handling and importing it in a browser costs nothing.
  return convertFileSrc(`${file.digest}/${file.name}`, SCHEME);
}

/**
 * Where a page an agent wrote is framed from.
 *
 * On a desktop the artifact origin is a loopback port of its own. Hosted, the
 * daemon serves the same document on its own origin, behind the token, and
 * the frame's `sandbox` is what keeps the page's origin opaque either way.
 */
export function artifactUrl(at: { port: number; id: string }): string {
  if (hosted) {
    return `${workspaceOrigin()}/v1/artifact/${at.id}?token=${encodeURIComponent(token())}`;
  }
  return `http://127.0.0.1:${at.port}/${at.id}`;
}

/**
 * Where a computer's screen is framed from.
 *
 * The runtime hands back an absolute loopback address on a desktop and a
 * relative one on a server, because only the page knows which origin it
 * reached the daemon at. The ticket in that path is for one sandbox's screen
 * and nothing else.
 */
export function screenUrl(vncUrl: string): string {
  return vncUrl.startsWith("/") ? `${workspaceOrigin()}${vncUrl}` : vncUrl;
}

/** Sizes the way a person says them, matching what an agent is told. */
export function readableSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  if (bytes >= 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${bytes} bytes`;
}

/**
 * Fetches a whole file and returns a URL a frame will accept.
 *
 * WebKit refuses a custom scheme as a frame source whatever the CSP says: not
 * as `guacfile:`, not as a host, not through `default-src`. An image and a
 * `fetch` are allowed, a frame is not, and a document's own viewer only runs in
 * a frame. So the bytes are fetched over the scheme as usual and handed on as a
 * blob, which is a source WebKit does accept.
 *
 * The caller owns what comes back and must revoke it: this is the one place in
 * the app where a file's bytes sit in the renderer, and they sit there for
 * exactly as long as something is drawing them.
 */
export async function localCopy(file: Attachment): Promise<string> {
  const response = await fetch(fileUrl(file));
  if (!response.ok) throw new Error(whyNot(response.status));
  return URL.createObjectURL(await response.blob());
}

/**
 * Why the file store turned a preview down, in words the operator can act on.
 *
 * `app.rs` answers a preview with one of three things and they mean different
 * things to the person reading: the store was not open yet, the bytes are not
 * there, or something else went wrong. "This file could not be read" covered
 * all three and told nobody which, so the one failure anybody has hit was also
 * the one nobody could diagnose.
 */
export function whyNot(status: number): string {
  if (status === 404) return "its contents are not in this workspace's file store";
  if (status === 503) return "the file store was still opening";
  return `the file store answered ${status}`;
}

/** What was read of a text file, and whether that was all of it. */
export interface FileText {
  text: string;
  trimmed: boolean;
}

/**
 * What has already been read, by content and length.
 *
 * A channel reloads whenever anything happens in it and hands the transcript
 * fresh objects each time, so a snippet that is not remembered is read again
 * for every message that arrives, and a range request is the one kind the
 * webview's own cache does not hold. Keyed by digest, which is only safe
 * because of what a digest is: the bytes behind one cannot change.
 *
 * Bounded because the full view reads a quarter of a megabyte at a time and a
 * session is long. The oldest goes, which is the least likely to be on screen.
 */
const READ = new Map<string, Promise<FileText>>();
const READ_LIMIT = 64;

/**
 * Reads the front of a text file.
 *
 * Asked for as a range, so a 20 MB log costs the two kilobytes on screen rather
 * than 20 MB in the renderer. The slice is applied again on this side: a range
 * is a request, not a guarantee, and the answer to one a server declines to
 * honour is the whole file.
 */
export function readFileText(file: Attachment, limit: number): Promise<FileText> {
  const key = `${file.digest}:${limit}`;
  const held = READ.get(key);
  if (held) return held;

  // The promise rather than its result, because the readers of one file arrive
  // together: a channel reload remounts every message at once, and a cache
  // that only fills on completion misses all of them.
  const reading = read(file, limit);
  reading.catch(() => READ.delete(key));

  if (READ.size >= READ_LIMIT) READ.delete(READ.keys().next().value as string);
  READ.set(key, reading);
  return reading;
}

async function read(file: Attachment, limit: number): Promise<FileText> {
  const response = await fetch(fileUrl(file), { headers: { Range: `bytes=0-${limit - 1}` } });
  if (!response.ok && response.status !== 206) {
    throw new Error(whyNot(response.status));
  }
  const text = await response.text();
  // Against the file's own size rather than the text's: a character can be
  // several bytes, so a cut made at a byte count says nothing about the length
  // of what came back.
  return { text: text.slice(0, limit), trimmed: file.bytes > limit || text.length > limit };
}
