import type { Attachment } from "./types";
export interface Draft {
  text: string;
  files: Attachment[];
}
export function readDraft(key: string): Draft {
  try {
    const value = JSON.parse(sessionStorage.getItem(key) ?? "null");
    if (
      value &&
      typeof value.text === "string" &&
      Array.isArray(value.files) &&
      value.files.every(
        (file: Attachment) =>
          typeof file.digest === "string" &&
          /^[a-f0-9]{64}$/.test(file.digest) &&
          typeof file.name === "string" &&
          typeof file.mime === "string" &&
          Number.isSafeInteger(file.bytes) &&
          file.bytes >= 0,
      )
    )
      return value;
  } catch {
    /* An unavailable session store does not prevent composition. */
  }
  return { text: "", files: [] };
}
export function saveDraft(key: string, draft: Draft) {
  if (draft.text || draft.files.length) sessionStorage.setItem(key, JSON.stringify(draft));
  else sessionStorage.removeItem(key);
}
