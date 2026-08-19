import { beforeEach, describe, expect, it, vi } from "vitest";

import { fileUrl, previewKind, readableSize, readFileText } from "./files";
import type { Attachment } from "./types";

/**
 * A stored file. The digest follows the name so that two different files in
 * this suite are two different files to the read cache, as they would be.
 */
function file(name: string, mime: string, bytes = 10): Attachment {
  return { digest: name.padEnd(64, "0").slice(0, 64), name, mime, bytes };
}

describe("previewKind", () => {
  it("draws pictures, documents and text, and says so for nothing else", () => {
    expect(previewKind("image/png")).toBe("image");
    expect(previewKind("image/webp")).toBe("image");
    expect(previewKind("application/pdf")).toBe("pdf");
    expect(previewKind("text/markdown")).toBe("text");
    expect(previewKind("application/json")).toBe("text");
  });

  it("refuses to guess at anything else", () => {
    // The same rule the runtime follows deciding what to show a model: a file
    // that is only mostly text renders as a screen of replacement characters,
    // and a row saying what it is serves the operator better.
    for (const mime of [
      "application/zip",
      "application/octet-stream",
      "application/vnd.ms-excel",
      "application/msword",
    ]) {
      expect(previewKind(mime), mime).toBe("none");
    }
  });
});

describe("fileUrl", () => {
  it("addresses the bytes by digest and carries the name for its type", () => {
    // The name is not what finds the file. Two agents sending one document
    // under different names are one set of bytes at one address.
    const stored = file("brief.pdf", "application/pdf");
    const url = fileUrl(stored);
    expect(url).toContain("guacfile:");
    expect(decodeURIComponent(url)).toContain(`${stored.digest}/brief.pdf`);
  });

  it("escapes a name, since a person's file is called what they called it", () => {
    const url = fileUrl(file("last quarter (final).pdf", "application/pdf"));
    expect(url).not.toContain(" ");
    expect(decodeURIComponent(url)).toContain("last quarter (final).pdf");
  });
});

describe("readableSize", () => {
  it("says sizes the way a person does, matching what an agent is told", () => {
    expect(readableSize(512)).toBe("512 bytes");
    expect(readableSize(2048)).toBe("2 KB");
    expect(readableSize(1024 * 1024 * 1.5)).toBe("1.5 MB");
  });
});

describe("readFileText", () => {
  const fetched = vi.fn();

  beforeEach(() => {
    fetched.mockReset();
    globalThis.fetch = fetched as unknown as typeof fetch;
  });

  function answers(body: string, status = 206) {
    fetched.mockResolvedValue({ ok: status === 200, status, text: async () => body });
  }

  it("asks for the front of the file rather than all of it", async () => {
    // A log nobody meant to attach is 20 MB, and the operator asked for a
    // glance at it, not for it to be in the renderer.
    answers("hello");
    await readFileText(file("big.log", "text/plain", 20_000_000), 2048);

    const asked = fetched.mock.calls[0] ?? [];
    expect(asked[1].headers.Range).toBe("bytes=0-2047");
  });

  it("cuts the answer itself, because a range is a request and not a promise", async () => {
    answers("0123456789");
    const read = await readFileText(file("notes.md", "text/markdown", 10), 4);

    expect(read.text).toBe("0123");
    expect(read.trimmed).toBe(true);
  });

  it("says when it read the whole thing", async () => {
    answers("short", 200);
    const read = await readFileText(file("notes.md", "text/markdown", 5), 2048);

    expect(read.text).toBe("short");
    expect(read.trimmed).toBe(false);
  });

  it("fails by name when the file cannot be read", async () => {
    fetched.mockResolvedValue({ ok: false, status: 404, text: async () => "" });
    await expect(readFileText(file("gone.txt", "text/plain"), 2048)).rejects.toThrow("gone.txt");
  });

  it("reads one file once, however many times the transcript redraws it", async () => {
    // A channel reload hands every message new objects, so this runs again for
    // every message that arrives, and a range request is the one kind the
    // webview's own cache does not hold.
    answers("cached");
    const twice = file("stable.md", "text/markdown");

    await readFileText(twice, 2048);
    await readFileText(twice, 2048);

    expect(fetched).toHaveBeenCalledTimes(1);
    // A different length is a different read, since it can be cut differently.
    await readFileText(twice, 64);
    expect(fetched).toHaveBeenCalledTimes(2);
  });
});
