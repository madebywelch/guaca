import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { Composer } from "./Composer";

/** The drop handlers the component registered, so a test can fire one. */
let dropped: ((paths: string[]) => void) | null = null;
let over: ((inside: boolean) => void) | null = null;

vi.mock("../lib/ipc", () => ({
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
    // they recognise.
    expect(screen.getByText("proposal.docx")).toBeTruthy();
    expect(screen.queryByText(/Users\/robert/)).toBeNull();

    await type("have a look");
    await click("Send");

    expect(onSend).toHaveBeenCalledWith("have a look", ["/Users/robert/Documents/proposal.docx"]);
  });

  it("lets a file be sent with nothing typed", async () => {
    // Handing over a document with no covering note is how people actually
    // send one, and Send is disabled on an empty box.
    const onSend = await draw();
    expect(screen.getByRole("button", { name: "Send" }).hasAttribute("disabled")).toBe(true);

    await drop(["/tmp/agenda.txt"]);
    await click("Send");

    expect(onSend).toHaveBeenCalledWith("", ["/tmp/agenda.txt"]);
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
      throw new Error("the file was too big");
    });
    render(<Composer placeholder="Message Manager" onSend={onSend} />);
    await waitFor(() => expect(dropped).not.toBeNull());

    await drop(["/tmp/enormous.zip"]);
    await type("here");
    await click("Send");

    await waitFor(() => expect(screen.getByText("enormous.zip")).toBeTruthy());
    expect((screen.getByRole("combobox") as HTMLTextAreaElement).value).toBe("here");
  });

  it("takes one copy of a file dropped twice", async () => {
    await draw();
    await drop(["/tmp/same.md"]);
    await drop(["/tmp/same.md"]);

    expect(screen.getAllByText("same.md")).toHaveLength(1);
  });

  it("removes one it was not meant to have", async () => {
    const onSend = await draw();
    await drop(["/tmp/wrong.md", "/tmp/right.md"]);
    await click("Remove wrong.md");

    expect(screen.queryByText("wrong.md")).toBeNull();
    await click("Send");
    expect(onSend).toHaveBeenCalledWith("", ["/tmp/right.md"]);
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
