import { convertFileSrc } from "@tauri-apps/api/core";

import { hosted, token, workspaceOrigin } from "./transport";
import type { Attachment } from "./types";

const SCHEME = "guacfile";

export const SNIPPET_BYTES = 2048;

export const PREVIEW_BYTES = 256 * 1024;

export type PreviewKind = "image" | "pdf" | "markdown" | "text" | "none";

export function previewKind(mime: string): PreviewKind {
  if (mime.startsWith("image/")) return "image";
  if (mime === "application/pdf") return "pdf";
  if (mime === "text/markdown") return "markdown";
  if (mime.startsWith("text/") || mime === "application/json") return "text";
  return "none";
}

export function kindLabel(mime: string): string {
  if (mime === "application/pdf") return "PDF";
  if (mime === "application/zip") return "ZIP archive";
  if (mime === "text/markdown") return "Markdown";
  if (mime.startsWith("image/")) return `${mime.slice("image/".length).toUpperCase()} image`;
  if (mime.startsWith("text/")) return mime.slice("text/".length).toUpperCase();
  if (mime === "application/json") return "JSON";
  if (mime.includes("wordprocessingml") || mime === "application/msword") return "Word document";
  if (mime.includes("spreadsheetml") || mime === "application/vnd.ms-excel") return "Spreadsheet";
  if (mime === "application/octet-stream") return "File";
  return mime;
}

export function fileUrl(file: Pick<Attachment, "digest" | "name">): string {
  if (hosted) {
    const target = `${encodeURIComponent(file.digest)}/${encodeURIComponent(file.name)}`;
    const ticket = token();
    return ticket
      ? `${workspaceOrigin()}/v1/file/${target}?token=${encodeURIComponent(ticket)}`
      : `${workspaceOrigin()}/v1/file/${target}`;
  }
  return convertFileSrc(`${file.digest}/${file.name}`, SCHEME);
}

export function artifactUrl(at: { port: number; id: string; ticket?: string | null }): string {
  if (hosted) {
    return `${workspaceOrigin()}/v1/artifact/${at.id}?token=${encodeURIComponent(at.ticket ?? "")}`;
  }
  return `http://127.0.0.1:${at.port}/${at.id}`;
}

export function screenUrl(vncUrl: string): string {
  return vncUrl.startsWith("/") ? `${workspaceOrigin()}${vncUrl}` : vncUrl;
}

export function readableSize(bytes: number): string {
  if (bytes >= 1024 * 1024) {
    const tenths = (bytes * 10) / (1024 * 1024);
    const below = Math.floor(tenths);
    const rounded =
      tenths - below > 0.5 || (tenths - below === 0.5 && below % 2 === 1) ? below + 1 : below;
    return `${Math.floor(rounded / 10)}.${rounded % 10} MB`;
  }
  if (bytes >= 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${bytes} bytes`;
}

export async function localCopy(file: Attachment): Promise<string> {
  const response = await fetch(fileUrl(file));
  if (!response.ok) throw new Error(whyNot(response.status));
  return URL.createObjectURL(await response.blob());
}

export function whyNot(status: number): string {
  if (status === 404) return "its contents are not in this host's file store";
  if (status === 503) return "the file store was still opening";
  return `the file store answered ${status}`;
}

export interface FileText {
  text: string;
  trimmed: boolean;
}

const READ = new Map<string, Promise<FileText>>();
const READ_LIMIT = 64;

export function readFileText(file: Attachment, limit: number): Promise<FileText> {
  const key = `${file.digest}:${limit}`;
  const held = READ.get(key);
  if (held) return held;

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
  return { text: text.slice(0, limit), trimmed: file.bytes > limit || text.length > limit };
}
