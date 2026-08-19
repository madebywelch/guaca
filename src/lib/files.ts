/**
 * How the webview gets at a file, and what it can show of one.
 *
 * The bytes never cross IPC: a transcript does, in bulk, which is the whole
 * reason a message carries a digest rather than a document. They come over
 * `guacfile:` instead, a scheme `app.rs` answers out of the file store, so a
 * picture is fetched once by the element drawing it and only while that element
 * is on screen.
 */

import { convertFileSrc } from "@tauri-apps/api/core";

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
export type PreviewKind = "image" | "pdf" | "text" | "none";

/**
 * Which of the four a file is.
 *
 * By type, never by sniffing the bytes, which is the same rule the runtime
 * follows when it decides what to show a model. A file that is only mostly text
 * renders as a screenful of replacement characters, and the operator is better
 * served by a row saying what it is and a button that saves it.
 */
export function previewKind(mime: string): PreviewKind {
  if (mime.startsWith("image/")) return "image";
  if (mime === "application/pdf") return "pdf";
  if (mime.startsWith("text/") || mime === "application/json") return "text";
  return "none";
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
  return convertFileSrc(`${file.digest}/${file.name}`, SCHEME);
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
  if (!response.ok) throw new Error(`could not read ${file.name}`);
  return URL.createObjectURL(await response.blob());
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
    throw new Error(`could not read ${file.name}`);
  }
  const text = await response.text();
  // Against the file's own size rather than the text's: a character can be
  // several bytes, so a cut made at a byte count says nothing about the length
  // of what came back.
  return { text: text.slice(0, limit), trimmed: file.bytes > limit || text.length > limit };
}
