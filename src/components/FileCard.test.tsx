import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { fileUrl } from "../lib/files";
import type { Attachment } from "../lib/types";
import { FileCard } from "./FileCard";

const saveFile = vi.fn(async (_digest: string, _name: string) => "/Users/robert/Downloads/x");
const setBanner = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: { saveFile: (digest: string, name: string) => saveFile(digest, name) },
}));

/** What the card reads off the store: one action, and whether there is a disk. */
const state = {
  setBanner,
  capabilities: {
    localDirectories: true,
    loopbackEndpoints: true,
    claudeProvider: true,
    claudeCodeHarness: true,
    localFiles: true,
  },
};

vi.mock("../lib/store", () => ({
  useStore: Object.assign((select: (s: typeof state) => unknown) => select(state), {
    getState: () => state,
  }),
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
    const { unmount, container } = render(<FileCard file={file("brief.pdf", "application/pdf")} />);

    // By tag rather than by title: the name button carries the whole file name
    // as a tooltip now, because it truncates.
    await waitFor(() => expect(container.querySelector("iframe")).toBeTruthy());
    const page = container.querySelector("iframe") as HTMLIFrameElement;
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

  it("says a document could not be read rather than leaving a gray box", async () => {
    fetched.mockResolvedValue({ ok: false, status: 404, blob: async () => new Blob([]) });
    render(<FileCard file={file("missing.pdf", "application/pdf")} />);

    // The reason, not just the fact: a preview that fails while the store is
    // still opening wants trying again, and one whose bytes are missing does
    // not, and "could not be read" said neither.
    await waitFor(() =>
      expect(screen.getByText(/not in this workspace's file store/)).toBeTruthy(),
    );
    expect(screen.getByRole("button", { name: "Try again" })).toBeTruthy();
  });

  it("reads the first lines of a text file into the transcript", async () => {
    const { container } = render(<FileCard file={file("server.log", "text/plain")} />);

    await waitFor(() => expect(screen.getByText(/first line/)).toBeTruthy());
    // As its own source. A log is not prose, and markdown rules applied to one
    // would eat its punctuation.
    expect(container.querySelector(".file__text")).toBeTruthy();
    const asked = fetched.mock.calls[0] ?? [];
    expect(asked[0]).toContain("guacfile:");
    expect(asked[1].headers.Range).toBe("bytes=0-2047");
  });

  it("draws a markdown file as the document it is, not as its source", async () => {
    // The format the agents actually write in. A brief shown as monospace `##`
    // is a document the operator reads around rather than through, and this app
    // already renders every message body as the prose it is.
    fetched.mockResolvedValue({
      ok: true,
      status: 206,
      text: async () => "# Risks\n\n- the vendor\n- **the deadline**\n",
    });
    const { container } = render(<FileCard file={file("brief.md", "text/markdown")} />);

    await waitFor(() => expect(container.querySelector("h1")).toBeTruthy());
    expect(container.querySelector("h1")?.textContent).toBe("Risks");
    expect(container.querySelectorAll("li")).toHaveLength(2);
    expect(container.querySelector("strong")?.textContent).toBe("the deadline");
    // Never the raw syntax, which is the whole point.
    expect(container.textContent).not.toContain("**");
    expect(container.querySelector(".file__text")).toBeNull();
  });

  it("renders no HTML out of a file, whoever's machine it came off", async () => {
    // A file from an agent's computer is no more trustworthy than the message
    // that carried it, and the message renderer has never allowed raw HTML.
    // `react-markdown` ignores it unless `rehype-raw` is added, and it is not.
    fetched.mockResolvedValue({
      ok: true,
      status: 206,
      text: async () => '<img src="x" onerror="alert(1)"><script>alert(2)</script>\n\nplain\n',
    });
    const { container } = render(<FileCard file={file("hostile.md", "text/markdown")} />);

    await waitFor(() => expect(container.textContent).toContain("plain"));
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
  });

  it("names anything it cannot draw and offers no preview it does not have", () => {
    render(<FileCard file={file("archive.zip", "application/zip")} />);

    expect(screen.getByText("archive.zip")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Open archive.zip" })).toBeNull();
    // The one row with no preview to speak for it, so it says what it is. A
    // name and a size and no other fact is what leaves an operator wondering
    // what the app thinks it is holding.
    expect(screen.getByText(/ZIP archive/)).toBeTruthy();
  });

  it("opens the full view on the name, whatever the file is", async () => {
    render(<FileCard file={file("archive.zip", "application/zip")} />);
    fireEvent.click(screen.getByRole("button", { name: "archive.zip" }));

    const dialog = screen.getByRole("dialog");
    expect(dialog.getAttribute("aria-label")).toBe("archive.zip");
    // Clicking a file that cannot be shown has to leave the operator somewhere
    // better than where they were: what it is, and the one thing that always
    // works on a file this app cannot open.
    expect(within(dialog).getByText(/ZIP archive/)).toBeTruthy();
    expect(within(dialog).getByRole("button", { name: "Save a copy" })).toBeTruthy();
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

    fireEvent.click(screen.getByRole("button", { name: "Save a copy" }));

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

  it("hands the browser a download where there is no downloads folder", () => {
    // On a server the box's downloads folder is nobody's. The browser has its
    // own, and the same bytes are already on the route every preview reads.
    state.capabilities = { ...state.capabilities, localFiles: false };
    try {
      render(<FileCard file={file("brief.pdf", "application/pdf")} />);

      expect(screen.queryByRole("button", { name: "Save a copy" })).toBeNull();
      const link = screen.getByRole("link", { name: "Download" }) as HTMLAnchorElement;
      expect(link.getAttribute("download")).toBe("brief.pdf");
      // The same address every preview reads from, whichever host spelled it.
      expect(link.getAttribute("href")).toBe(fileUrl(file("brief.pdf", "application/pdf")));
      expect(saveFile).not.toHaveBeenCalled();
    } finally {
      state.capabilities = { ...state.capabilities, localFiles: true };
    }
  });

  it("says why a copy could not be saved", async () => {
    saveFile.mockRejectedValue({ kind: "file", message: "no file here with that content" });
    render(<FileCard file={file("brief.pdf", "application/pdf")} />);

    fireEvent.click(screen.getByRole("button", { name: "Save a copy" }));

    await waitFor(() =>
      expect(setBanner).toHaveBeenCalledWith({
        tone: "error",
        text: "no file here with that content",
      }),
    );
  });

  it("says a text file could not be read rather than showing an empty box", async () => {
    fetched.mockRejectedValue(new Error("the file store was still opening"));
    render(<FileCard file={file("unreadable.md", "text/markdown")} />);

    await waitFor(() =>
      expect(
        screen.getByText(/Could not read this file: the file store was still opening/),
      ).toBeTruthy(),
    );
  });

  it("offers another go at a read that failed, and takes it", async () => {
    // The commonest failure here is one that has already stopped being true by
    // the time it is on screen: the store answers 503 while it is opening, and
    // a preview that mounted in that window used to stay broken for the life of
    // the channel.
    // Its own name, so its own digest: the read cache is keyed by content and
    // outlives a test, which is the whole point of it everywhere else.
    fetched.mockRejectedValueOnce(new Error("the file store was still opening"));
    const { container } = render(<FileCard file={file("retry.md", "text/markdown")} />);

    const retry = await screen.findByRole("button", { name: "Try again" });
    fireEvent.click(retry);

    await waitFor(() =>
      expect(container.querySelector(".file__doc")?.textContent).toContain("first line"),
    );
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
  });
});
