import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, Attachment, Staged } from "../lib/types";
import { Composer } from "./Composer";

/** The drop handlers the component registered, so a test can fire one. */
let dropped: ((staged: Promise<Staged>) => void) | null = null;
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

const uploads = vi.fn<(files: File[]) => Promise<Staged>>();

vi.mock("../lib/ipc", () => ({
  api: {
    stageFiles: (paths: string[]) => staging(paths),
    stageUploads: (files: File[]) => uploads(files),
  },
  onFileDrop: async (handlers: {
    dropped: (staged: Promise<Staged>) => void;
    over: (inside: boolean) => void;
  }) => {
    dropped = handlers.dropped;
    over = handlers.over;
    return () => {};
  },
}));

/** Every live agent in the workspace, which is more than one crew. */
let roster: AgentCard[] = [];

/** What the composer reads off the store: the roster, and whether there is a disk. */
const state = {
  capabilities: {
    localDirectories: true,
    loopbackEndpoints: true,
    claudeProvider: true,
    claudeCodeHarness: true,
    localFiles: true,
  },
};

vi.mock("../lib/store", () => ({
  useLiveAgents: () => roster,
  useStore: (select: (s: typeof state) => unknown) => select(state),
}));

/** The crew whose channel is open in these tests. */
const CREW = "00000000-0000-4000-8000-000000000001";
/** Another one, whose names the box must never offer. */
const ELSEWHERE = "00000000-0000-4000-8000-000000000002";

/** Minted per card rather than per name: two crews can hold one name. */
let minted = 0;

/** An agent as the rail hands one over: only the fields the composer draws. */
function anAgent(name: string, group = CREW): AgentCard {
  return {
    id: `00000000-0000-4000-8000-${(++minted).toString().padStart(12, "0")}`,
    groupId: group,
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    browserConsent: "open",
    repositoryId: null,
    version: 1,
  };
}

describe("Composer", () => {
  beforeEach(() => {
    dropped = null;
    over = null;
    roster = [];
    staging = async (paths) => ({ attached: paths.map(stored), refused: [] });
  });

  async function draw(onSend = vi.fn(async () => {})) {
    render(<Composer placeholder="Message Manager" group={CREW} onSend={onSend} />);
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
    render(<Composer placeholder="Message Manager" group={CREW} onSend={onSend} />);
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

  it("offers a file button where there are no paths to drop, and takes what it picks", async () => {
    // A browser has bytes rather than paths, so the drop is joined by a
    // picker. On a desktop the button is not there: the window takes a path
    // and the runtime reads the file.
    state.capabilities = { ...state.capabilities, localFiles: false };
    uploads.mockResolvedValue({ attached: [stored("brief.pdf")], refused: [] });
    try {
      const onSend = await draw();
      const picker = document.querySelector('input[type="file"]') as HTMLInputElement;
      expect(picker).toBeTruthy();
      expect(screen.getByRole("button", { name: "Attach a file" })).toBeTruthy();

      const chosen = new File(["%PDF"], "brief.pdf", { type: "application/pdf" });
      await act(async () => {
        Object.defineProperty(picker, "files", { value: [chosen], configurable: true });
        fireEvent.change(picker);
      });

      expect(uploads).toHaveBeenCalledWith([chosen]);
      await waitFor(() => expect(screen.getByText("brief.pdf")).toBeTruthy());
      await click("Send");
      expect(onSend).toHaveBeenCalledWith("", [stored("brief.pdf")]);
    } finally {
      state.capabilities = { ...state.capabilities, localFiles: true };
    }
  });

  it("has no file button on a desktop", async () => {
    await draw();
    expect(screen.queryByRole("button", { name: "Attach a file" })).toBeNull();
  });

  it("says where a dragged file will land while it is over the window", async () => {
    await draw();
    await act(async () => {
      over?.(true);
    });
    expect(screen.getByText("Drop to attach")).toBeTruthy();
  });
});

describe("a mention in the box", () => {
  /** The layer under the textarea, which is where a draft's mentions are drawn. */
  const painted = () => document.querySelector(".composer__mirror");

  async function draw(group: string | null = CREW) {
    render(<Composer placeholder="Message Manager" group={group} onSend={vi.fn(async () => {})} />);
    await waitFor(() => expect(dropped).not.toBeNull());
  }

  /** The names the typeahead is offering right now. */
  const offered = () =>
    [...document.querySelectorAll(".mentions__name")].map((item) => item.textContent);

  it("offers the channel's own crew and nobody else", async () => {
    // The workspace-wide list is what an operator sees after clearing a crew
    // and hiring a new one: every agent they have ever had, on a menu where
    // picking one writes a name `send_message` refuses as belonging to nobody.
    roster = [anAgent("Critic"), anAgent("Scribe", ELSEWHERE)];
    await draw();
    await type("ask @");

    expect(offered()).toEqual(["Critic"]);
  });

  it("offers nobody when the channel belongs to nobody", async () => {
    roster = [anAgent("Critic")];
    await draw(null);
    await type("ask @");

    expect(offered()).toEqual([]);
  });

  it("marks a name the crew has, as it is typed", async () => {
    roster = [anAgent("Critic"), anAgent("Head Chef")];
    await draw();
    await type("ask @Critic and @Head Chef about @lunch");

    const chips = [...(painted()?.querySelectorAll(".mention") ?? [])];
    expect(chips.map((chip) => chip.getAttribute("data-mention"))).toEqual(["Critic", "Head Chef"]);
  });

  it("paints exactly the characters the box holds, and no others", async () => {
    // The layer sits under the operator's own text, so a copy that is one
    // character out is a pill beside the name instead of behind it. Nothing in
    // a window with no layout can see that; that the two strings agree is what
    // can be checked here, and `styles.test.ts` holds the metrics.
    roster = [anAgent("Critic")];
    await draw();
    await type("ask @Critic to review\nand say so");

    const box = screen.getByRole("combobox") as HTMLTextAreaElement;
    expect(painted()?.textContent).toBe(box.value);
  });

  it("is the same text twice, so only one of them is read out", async () => {
    roster = [anAgent("Critic")];
    await draw();
    await type("@Critic");

    expect(painted()?.getAttribute("aria-hidden")).toBe("true");
  });

  it("marks nothing when the name is nobody's", async () => {
    roster = [anAgent("Critic")];
    await draw();
    await type("mail bob@example.com about @lunch");

    expect(painted()?.querySelectorAll(".mention")).toHaveLength(0);
  });

  it("marks nothing when the name belongs to another crew", async () => {
    // The chip is the promise the menu made, one step later: drawing one
    // around a name in a crew this channel cannot reach says the message is
    // addressed to somebody it will never arrive at.
    roster = [anAgent("Critic"), anAgent("Scribe", ELSEWHERE)];
    await draw();
    await type("ask @Scribe about it");

    expect(painted()?.querySelectorAll(".mention")).toHaveLength(0);
  });

  it("marks the name the typeahead just completed", async () => {
    // The two have to agree: a completion the operator accepted that then drew
    // as prose reads as the app having refused it.
    roster = [anAgent("Head Chef")];
    await draw();
    await type("ask @Head");
    await act(async () => {
      fireEvent.keyDown(screen.getByRole("combobox"), { key: "Enter" });
    });

    await waitFor(() =>
      expect(painted()?.querySelector(".mention")?.getAttribute("data-mention")).toBe("Head Chef"),
    );
  });

  it("lets go of a mention that stops being one", async () => {
    roster = [anAgent("Critic")];
    await draw();
    await type("@Critic");
    expect(painted()?.querySelectorAll(".mention")).toHaveLength(1);

    await type("@Critical");
    expect(painted()?.querySelectorAll(".mention")).toHaveLength(0);
  });
});

/** A drop arrives from the runtime, not from the DOM, so it is fired by hand.
 *  What the composer is handed is the store's answer, already under way. */
async function drop(paths: string[]) {
  await act(async () => {
    dropped?.(staging(paths));
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
