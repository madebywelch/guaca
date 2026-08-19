import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Attachment } from "../lib/types";
import { FileCard } from "./FileCard";

const saveFile = vi.fn(async (_digest: string, _name: string) => "/Users/robert/Downloads/x");
const setBanner = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: { saveFile: (digest: string, name: string) => saveFile(digest, name) },
}));

vi.mock("../lib/store", () => ({
  useStore: { getState: () => ({ setBanner }) },
}));

/**
 * A stored file. The digest follows the name, so two files in this suite are
 * two files to the read cache, as they would be anywhere else.
 */
function file(name: string, mime: string, bytes = 2048): Attachment {
  return { digest: name.padEnd(64, "0").slice(0, 64), name, mime, bytes };
}

const fetched = vi.fn();
const made: string[] = [];
const revoked: string[] = [];

describe("FileCard", () => {
  beforeEach(() => {
    saveFile.mockClear();
    setBanner.mockClear();
    fetched.mockReset();
    fetched.mockResolvedValue({
      ok: true,
      status: 206,
      text: async () => "first line\nsecond",
      blob: async () => new Blob(["%PDF-1.7"]),
    });
    globalThis.fetch = fetched as unknown as typeof fetch;

    // jsdom has no object URLs. A document is handed to its frame as one,
    // because WebKit will not take a custom scheme there.
    made.length = 0;
    revoked.length = 0;
    URL.createObjectURL = vi.fn(() => {
      const url = `blob:guaca/${made.length}`;
      made.push(url);
      return url;
    });
    URL.revokeObjectURL = vi.fn((url: string) => void revoked.push(url));
  });

  it("draws a picture rather than naming one", async () => {
    render(<FileCard file={file("shot.png", "image/png")} />);

    const drawn = screen.getByAltText("shot.png") as HTMLImageElement;
    const stored = file("shot.png", "image/png");
    expect(drawn.src).toContain(encodeURIComponent(`${stored.digest}/shot.png`));
    // Three hundred messages open with a channel and the operator is looking
    // at the last few.
    expect(drawn.getAttribute("loading")).toBe("lazy");
    expect(screen.getByText("2 KB")).toBeTruthy();
  });

  it("gives a document its first page, in a frame that does not take the click", async () => {
    const { unmount } = render(<FileCard file={file("brief.pdf", "application/pdf")} />);

    const page = (await screen.findByTitle("brief.pdf")) as HTMLIFrameElement;
    // A copy of the document, not its address: WebKit refuses a custom scheme
    // as a frame source whatever the CSP says, and a document's own viewer only
    // runs in a frame. The bytes still arrive over the scheme.
    expect(page.src).toBe(made[0]);
    expect(fetched.mock.calls[0]?.[0]).toContain(encodeURIComponent("brief.pdf"));
    // Focus and clicks belong to the button behind it, which is what opens the
    // document. A frame that swallowed either would strand the file.
    expect(page.getAttribute("tabindex")).toBe("-1");

    // The one place a file's bytes sit in the renderer, and a transcript
    // scrolls past a lot of documents.
    unmount();
    expect(revoked).toEqual([made[0]]);
  });

  it("says a document could not be read rather than leaving a grey box", async () => {
    fetched.mockResolvedValue({ ok: false, status: 404, blob: async () => new Blob([]) });
    render(<FileCard file={file("missing.pdf", "application/pdf")} />);

    await waitFor(() => expect(screen.getByText(/could not be read/)).toBeTruthy());
  });

  it("reads the first lines of a text file into the transcript", async () => {
    render(<FileCard file={file("notes.md", "text/markdown")} />);

    await waitFor(() => expect(screen.getByText(/first line/)).toBeTruthy());
    const asked = fetched.mock.calls[0] ?? [];
    expect(asked[0]).toContain("guacfile:");
    expect(asked[1].headers.Range).toBe("bytes=0-2047");
  });

  it("names anything it cannot draw and offers no preview it does not have", () => {
    render(<FileCard file={file("archive.zip", "application/zip")} />);

    expect(screen.getByText("archive.zip")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open archive.zip" })).toBeNull();
  });

  it("opens the full view on the name, whatever the file is", async () => {
    render(<FileCard file={file("archive.zip", "application/zip")} />);
    fireEvent.click(screen.getByRole("button", { name: "archive.zip" }));

    const dialog = screen.getByRole("dialog");
    expect(dialog.getAttribute("aria-label")).toBe("archive.zip");
    // Clicking a file that cannot be shown has to leave the operator somewhere
    // better than where they were.
    expect(screen.getByText(/Save a copy/)).toBeTruthy();
  });

  it("closes the full view on Escape", () => {
    render(<FileCard file={file("shot.png", "image/png")} />);
    fireEvent.click(screen.getByRole("button", { name: "Open shot.png" }));
    expect(screen.getByRole("dialog")).toBeTruthy();

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("saves a copy and says where it went", async () => {
    saveFile.mockResolvedValue("/Users/robert/Downloads/brief.pdf");
    render(<FileCard file={file("brief.pdf", "application/pdf")} />);

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(saveFile).toHaveBeenCalledWith(
        file("brief.pdf", "application/pdf").digest,
        "brief.pdf",
      ),
    );
    // A file saved somewhere the operator has to go looking for has not really
    // been saved.
    expect(setBanner).toHaveBeenCalledWith({
      tone: "ok",
      text: "Saved to /Users/robert/Downloads/brief.pdf",
    });
  });

  it("says why a copy could not be saved", async () => {
    saveFile.mockRejectedValue({ kind: "file", message: "no file here with that content" });
    render(<FileCard file={file("brief.pdf", "application/pdf")} />);

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(setBanner).toHaveBeenCalledWith({
        tone: "error",
        text: "no file here with that content",
      }),
    );
  });

  it("says a text file could not be read rather than showing an empty box", async () => {
    fetched.mockRejectedValue(new Error("gone"));
    render(<FileCard file={file("unreadable.md", "text/markdown")} />);

    await waitFor(() => expect(screen.getByText(/could not be read/)).toBeTruthy());
  });
});
