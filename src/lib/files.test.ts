import { beforeEach, describe, expect, it, vi } from "vitest";

import { fileUrl, kindLabel, previewKind, readableSize, readFileText } from "./files";
import type { Attachment } from "./types";

function file(name: string, mime: string, bytes = 10): Attachment {
  return { digest: name.padEnd(64, "0").slice(0, 64), name, mime, bytes };
}

describe("previewKind", () => {
  it("draws pictures, documents and text, and says so for nothing else", () => {
    expect(previewKind("image/png")).toBe("image");
    expect(previewKind("image/webp")).toBe("image");
    expect(previewKind("application/pdf")).toBe("pdf");
    expect(previewKind("application/json")).toBe("text");
  });

  it("gives markdown its own kind, because that is what the agents write in", () => {
    expect(previewKind("text/markdown")).toBe("markdown");
    expect(previewKind("text/plain")).toBe("text");
    expect(previewKind("text/csv")).toBe("text");
    expect(previewKind("text/html")).toBe("text");
  });

  it("says Markdown rather than shouting it", () => {
    expect(kindLabel("text/markdown")).toBe("Markdown");
    expect(kindLabel("text/csv")).toBe("CSV");
  });

  it("refuses to guess at anything else", () => {
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
    const stored = file("brief.pdf", "application/pdf");
    const url = fileUrl(stored);
    expect(url).toContain("/v1/file/");
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

  it("fails with the reason, because the name is already on the row", async () => {
    fetched.mockResolvedValue({ ok: false, status: 404, text: async () => "" });
    await expect(readFileText(file("gone.txt", "text/plain"), 2048)).rejects.toThrow(
      "not in this host's file store",
    );

    fetched.mockResolvedValue({ ok: false, status: 503, text: async () => "" });
    await expect(readFileText(file("early.txt", "text/plain"), 2048)).rejects.toThrow(
      "still opening",
    );
  });

  it("reads one file once, however many times the transcript redraws it", async () => {
    answers("cached");
    const twice = file("stable.md", "text/markdown");

    await readFileText(twice, 2048);
    await readFileText(twice, 2048);

    expect(fetched).toHaveBeenCalledTimes(1);
    await readFileText(twice, 64);
    expect(fetched).toHaveBeenCalledTimes(2);
  });
});

describe("where a frame points", () => {
  it("frames a page from the artifact origin on a desktop", async () => {
    const { artifactUrl } = await import("./files");
    expect(artifactUrl({ port: 4242, id: "abc" })).toBe(
      `${window.location.origin}/v1/artifact/abc?token=`,
    );
  });

  it("leaves an absolute screen address alone and resolves a relative one", async () => {
    const { screenUrl } = await import("./files");
    expect(screenUrl("http://127.0.0.1:5000/sbx/6080/viewer.html")).toBe(
      "http://127.0.0.1:5000/sbx/6080/viewer.html",
    );
    expect(screenUrl("/v1/screen/t/sbx/6080/viewer.html?a=1")).toBe(
      `${window.location.origin}/v1/screen/t/sbx/6080/viewer.html?a=1`,
    );
  });
});
