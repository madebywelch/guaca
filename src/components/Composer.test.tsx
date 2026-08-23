import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Attachment, Staged } from "../lib/types";
import { Composer } from "./Composer";

/** The drop handlers the component registered, so a test can fire one. */
let dropped: ((paths: string[]) => void) | null = null;
let over: ((inside: boolean) => void) | null = null;

/** What the runtime says came of a drop. Set per test where it matters. */
let staging: (paths: string[]) => Promise<Staged> = async (paths) => ({
  attached: paths.map(stored),
  refused: [],
});

/** A file as the store would hand it back: named, typed, and addressed. */
function stored(path: string): Attachment {
  const name = path.split("/").pop() ?? path;
  return {
    // Content addressing, faked by the name: two copies of one document in
    // different folders are one attachment, which is the case worth testing.
    digest: `digest-of-${name}`,
    name,
    mime: name.endsWith(".png") ? "image/png" : "text/plain",
    bytes: 12,
  };
}

vi.mock("../lib/ipc", () => ({
  api: { stageFiles: (paths: string[]) => staging(paths) },
  onFileDrop: async (handlers: {
    dropped: (paths: string[]) => void;
    over: (inside: boolean) => void;
  }) => {
    dropped = handlers.dropped;
    over = handlers.over;
    return () => {};
  },
}));

vi.mock("../lib/store", () => ({ useLiveAgents: () => [] }));

describe("Composer", () => {
  beforeEach(() => {
    dropped = null;
    over = null;
    staging = async (paths) => ({ attached: paths.map(stored), refused: [] });
  });

  async function draw(onSend = vi.fn(async () => {})) {
    render(<Composer placeholder="Message Manager" onSend={onSend} />);
    await waitFor(() => expect(dropped).not.toBeNull());
    return onSend;
  }

  it("shows a dropped file by name and sends it with the message", async () => {
    const onSend = await draw();
    await drop(["/Users/robert/Documents/proposal.docx"]);

    // The path is the operator's business and is never shown; the name is what
    // they recognize.
    expect(screen.getByText("proposal.docx")).toBeTruthy();
    expect(screen.queryByText(/Users\/robert/)).toBeNull();

    await type("have a look");
    await click("Send");

    expect(onSend).toHaveBeenCalledWith("have a look", [stored("proposal.docx")]);
  });

  it("lets a file be sent with nothing typed", async () => {
    // Handing over a document with no covering note is how people actually
    // send one, and Send is disabled on an empty box.
    const onSend = await draw();
    expect(screen.getByRole("button", { name: "Send" }).hasAttribute("disabled")).toBe(true);

    await drop(["/tmp/agenda.txt"]);
    await click("Send");

    expect(onSend).toHaveBeenCalledWith("", [stored("agenda.txt")]);
  });

  it("drops the attachments once they have gone", async () => {
    const onSend = await draw();
    await drop(["/tmp/one.md"]);
    await click("Send");

    await waitFor(() => expect(screen.queryByText("one.md")).toBeNull());
    expect(onSend).toHaveBeenCalledTimes(1);
  });

  it("keeps them when the send fails, so nothing has to be found again", async () => {
    const onSend = vi.fn(async () => {
      throw new Error("that agent has been deleted");
    });
    render(<Composer placeholder="Message Manager" onSend={onSend} />);
    await waitFor(() => expect(dropped).not.toBeNull());

    await drop(["/tmp/notes.md"]);
    await type("here");
    await click("Send");

    await waitFor(() => expect(screen.getByText("notes.md")).toBeTruthy());
    expect((screen.getByRole("combobox") as HTMLTextAreaElement).value).toBe("here");
  });

  it("takes one copy of a document dropped twice from two places", async () => {
    // By content, not by path. A second drop is a person making sure, and one
    // document saved in two folders is still one document.
    await draw();
    await drop(["/tmp/same.md"]);
    await drop(["/Users/robert/Desktop/same.md"]);

    expect(screen.getAllByText("same.md")).toHaveLength(1);
  });

  it("removes one it was not meant to have", async () => {
    const onSend = await draw();
    await drop(["/tmp/wrong.md", "/tmp/right.md"]);
    await click("Remove wrong.md");

    expect(screen.queryByText("wrong.md")).toBeNull();
    await click("Send");
    expect(onSend).toHaveBeenCalledWith("", [stored("right.md")]);
  });

  it("names the file it could not take and keeps the ones it could", async () => {
    // Refused on the drop rather than on the send: the operator is still
    // holding the file, and one document over the limit must not cost them the
    // message they have written or the four attachments beside it.
    staging = async () => ({
      attached: [stored("/tmp/brief.md")],
      refused: ["enormous.zip is 40000000 bytes, and the limit is 26214400"],
    });
    const onSend = await draw();
    await drop(["/tmp/brief.md", "/tmp/enormous.zip"]);

    expect(screen.getByText(/enormous.zip is 40000000 bytes/)).toBeTruthy();
    expect(screen.getByText("brief.md")).toBeTruthy();

    await click("Send");
    expect(onSend).toHaveBeenCalledWith("", [stored("brief.md")]);
  });

  it("says so when the drop itself failed", async () => {
    staging = async () => {
      throw new Error("could not use the file store");
    };
    await draw();
    await drop(["/tmp/anything.md"]);

    await waitFor(() => expect(screen.getByText(/could not use the file store/)).toBeTruthy());
  });

  it("shows a dropped picture rather than only its name", async () => {
    // Three screenshots dropped together are three copies of one long
    // timestamped name, and nothing else to tell them apart.
    await draw();
    await drop(["/tmp/shot.png"]);

    const thumbnail = document.querySelector(".chip__thumb") as HTMLImageElement | null;
    expect(thumbnail?.getAttribute("src")).toContain("digest-of-shot.png");
  });

  it("says where a dragged file will land while it is over the window", async () => {
    await draw();
    await act(async () => {
      over?.(true);
    });
    expect(screen.getByText("Drop to attach")).toBeTruthy();
  });
});

/** A drop arrives from the runtime, not from the DOM, so it is fired by hand. */
async function drop(paths: string[]) {
  await act(async () => {
    dropped?.(paths);
  });
}

async function click(name: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name }));
  });
}

async function type(text: string) {
  await act(async () => {
    fireEvent.change(screen.getByRole("combobox"), { target: { value: text } });
  });
}
