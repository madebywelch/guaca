import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import { arrived, CAP, crowding, Memory } from "./Memory";

const agentMemory = vi.fn<(id: string) => Promise<string>>();
const setAgentMemory = vi.fn<(id: string, content: string) => Promise<string>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentMemory: (id: string) => agentMemory(id),
    setAgentMemory: (id: string, content: string) => setAgentMemory(id, content),
  },
}));

/** The box, addressed the way a screen reader would. */
function box(): HTMLTextAreaElement {
  return screen.getByLabelText("Memory") as HTMLTextAreaElement;
}

function type(text: string) {
  fireEvent.change(box(), { target: { value: text } });
}

describe("Memory", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    agentMemory.mockResolvedValue("");
    setAgentMemory.mockImplementation(async (_id, content) => content.trim());
    useStore.setState({ memoryVersion: {} });
  });

  it("reads without a textarea and Cancel restores the stored page", async () => {
    agentMemory.mockResolvedValue("Original page");
    render(<Memory agentId="a1" />);
    expect(await screen.findByText("Original page")).toBeTruthy();
    expect(screen.queryByRole("textbox", { name: "Memory" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    type("Changed page");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.getByText("Original page")).toBeTruthy();
    expect(setAgentMemory).not.toHaveBeenCalled();
  });

  it("warns near the cap only while editing", async () => {
    agentMemory.mockResolvedValue("a".repeat(CAP - 100));
    render(<Memory agentId="a1" />);
    await screen.findByText("a".repeat(CAP - 100));
    expect(screen.queryByText("100 characters left.")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    expect(screen.getByText("100 characters left.")).toBeTruthy();
  });

  it("draws what the agent remembers, as the characters it was given", async () => {
    agentMemory.mockResolvedValue("# Style\nTerse. No preamble.");
    render(<Memory agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    await waitFor(() => expect(box().value).toBe("# Style\nTerse. No preamble."));
  });

  it("says there is nothing in one line, and explains itself in the title", async () => {
    // What the store is for used to be three sentences under every empty
    // memory, which is a paragraph the operator reads on every new agent and
    // never again needs. The section says what it holds; the explanation is on
    // the heading, read when somebody wonders rather than every time they look.
    render(<Memory agentId="a1" />);
    expect(await screen.findByText("Nothing yet.")).toBeTruthy();
    expect(screen.queryByText(/update_memory/)).toBeNull();
    expect(screen.getByRole("heading", { name: "Memory" }).getAttribute("title")).toMatch(
      /across conversations/,
    );

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    type("Smith handles verification.");
    expect(screen.queryByText("Nothing yet.")).toBeNull();
  });

  it("offers nothing to press until something has actually changed", async () => {
    agentMemory.mockResolvedValue("kept");
    render(<Memory agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    await waitFor(() => expect(box().value).toBe("kept"));
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);

    type("kept and more");
    expect(screen.getByRole("button", { name: "Save" })).toBeTruthy();

    // Typed back to what is on disk is not an edit.
    type("kept");
    expect((screen.getByRole("button", { name: "Save" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("puts back what was stored rather than what was typed", async () => {
    // The runtime trims and cuts, so leaving what was typed on screen would
    // show the operator a page their agent is never going to be given.
    setAgentMemory.mockResolvedValue("Smith handles verification.");
    render(<Memory agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    await waitFor(() => expect(box().value).toBe(""));

    type("  Smith handles verification.\n\n");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(screen.getByText("Smith handles verification.")).toBeTruthy());
    expect(screen.queryByRole("textbox", { name: "Memory" })).toBeNull();
    expect(setAgentMemory).toHaveBeenCalledWith("a1", "  Smith handles verification.\n\n");
    // Trimming is not a cut, and reporting it as one would cry wolf on every
    // save that ended with a newline.
    expect(screen.queryByText(/end was cut/)).toBeNull();
  });

  it("says so when the runtime kept less than was sent", async () => {
    setAgentMemory.mockResolvedValue("kept");
    render(<Memory agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    await waitFor(() => expect(box().value).toBe(""));

    type("kept and a great deal more");
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    expect(await screen.findByText(/end was cut/)).toBeTruthy();
    expect(screen.getByText("kept")).toBeTruthy();
  });

  it("reads itself again when the agent rewrites its own memory", async () => {
    // The operator reading a memory is most likely to be doing it while the
    // agent is working, which is exactly when the agent rewrites the file.
    render(<Memory agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    await waitFor(() => expect(box().value).toBe(""));

    agentMemory.mockResolvedValue("Smith verifies.");
    act(() => {
      useStore.getState().applyEvent({ type: "memoryChanged", agentId: "a1" });
    });

    await waitFor(() => expect(box().value).toBe("Smith verifies."));
  });

  it("ignores a rewrite belonging to another agent", async () => {
    render(<Memory agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    await waitFor(() => expect(agentMemory).toHaveBeenCalledTimes(1));

    act(() => {
      useStore.getState().applyEvent({ type: "memoryChanged", agentId: "a2" });
    });

    expect(agentMemory).toHaveBeenCalledTimes(1);
  });

  it("never takes a sentence away from the operator writing it", async () => {
    render(<Memory agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    await waitFor(() => expect(box().value).toBe(""));
    type("Smith is the one who ver");

    agentMemory.mockResolvedValue("The agent's own version.");
    act(() => {
      useStore.getState().applyEvent({ type: "memoryChanged", agentId: "a1" });
    });

    expect(await screen.findByText(/rewrote this while you were typing/)).toBeTruthy();
    expect(box().value).toBe("Smith is the one who ver");
  });

  it("hands over the version it held back when the operator drops their own", async () => {
    render(<Memory agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    await waitFor(() => expect(box().value).toBe(""));
    type("half a thought");

    agentMemory.mockResolvedValue("The agent's own version.");
    act(() => {
      useStore.getState().applyEvent({ type: "memoryChanged", agentId: "a1" });
    });
    await screen.findByText(/rewrote this while you were typing/);

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(screen.getByText("The agent's own version.")).toBeTruthy();
    expect(screen.queryByText(/rewrote this while you were typing/)).toBeNull();
  });

  it("keeps the operator's own when they save over a rewrite", async () => {
    render(<Memory agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    await waitFor(() => expect(box().value).toBe(""));
    type("mine");

    agentMemory.mockResolvedValue("theirs");
    act(() => {
      useStore.getState().applyEvent({ type: "memoryChanged", agentId: "a1" });
    });
    await screen.findByText(/rewrote this while you were typing/);

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(setAgentMemory).toHaveBeenCalledWith("a1", "mine"));
    expect(screen.getByText("mine")).toBeTruthy();
    expect(screen.queryByText(/rewrote this while you were typing/)).toBeNull();
  });

  it("draws the read that failed rather than an empty memory", async () => {
    // An empty box is a claim that the agent remembers nothing, which is a
    // different thing from not having been able to find out.
    agentMemory.mockRejectedValue(new Error("could not access the workspace"));
    render(<Memory agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));

    expect(await screen.findByText(/could not access the workspace/)).toBeTruthy();
  });
});

describe("arrived", () => {
  it("applies a read when nothing is being written", () => {
    expect(arrived(null, "first")).toEqual({ stored: "first", draft: null, incoming: null });
    expect(arrived({ stored: "old", draft: null, incoming: null }, "new")).toEqual({
      stored: "new",
      draft: null,
      incoming: null,
    });
  });

  it("holds a read back from a draft rather than replacing it", () => {
    expect(arrived({ stored: "old", draft: "mine", incoming: null }, "theirs")).toEqual({
      stored: "old",
      draft: "mine",
      incoming: "theirs",
    });
  });

  it("applies it over a draft typed back to what is on disk", () => {
    // Otherwise the panel sits on a page the agent has replaced, with nothing
    // of the operator's to lose by moving on from it.
    expect(arrived({ stored: "same", draft: "same", incoming: null }, "new")).toEqual({
      stored: "new",
      draft: null,
      incoming: null,
    });
  });

  it("keeps the newest held version rather than stacking them", () => {
    const held = arrived({ stored: "old", draft: "mine", incoming: "first" }, "second");
    expect(held.incoming).toBe("second");
  });
});

describe("crowding", () => {
  // Written against `CAP` rather than against the number it currently holds.
  // These were spelled 3_900 and 4_050 and had to be rewritten the day the
  // runtime went to four pages, which is the same drift the pinning test below
  // exists to stop: a suite that hardcodes somebody else's constant is one more
  // copy of it.
  it("says nothing about a memory with room in it", () => {
    expect(crowding("")).toBeNull();
    expect(crowding("a".repeat(CAP - 1_000))).toBeNull();
  });

  it("counts down the last of the room as a fact", () => {
    const room = crowding("a".repeat(CAP - 100));
    expect(room?.over).toBe(false);
    expect(room?.text).toBe("100 characters left.");
  });

  it("calls going over what it is, which is a loss", () => {
    const room = crowding("a".repeat(CAP + 50));
    expect(room?.over).toBe(true);
    expect(room?.text).toBe("50 characters over. The end is cut on save.");
  });

  it("counts what the runtime counts, not what `length` counts", () => {
    // `Workspace::write` cuts on Unicode scalar values. A page of emoji is half
    // the characters JavaScript reports, and warning at half the cap would be a
    // warning about nothing.
    expect(crowding("🥑".repeat(CAP - 1_000))).toBeNull();
  });

  it("warns against the cap the runtime actually enforces", () => {
    // The one number in this file that is somebody else's. Read out of the
    // Rust rather than trusted, because the two sides drifted to 4,000 against
    // 16,000 once already, and the panel spent that release telling operators
    // their memory was about to be cut by a runtime that was storing it whole.
    // Nothing else in the build compares them: both sides compile, both sides
    // pass, and the only symptom is a sentence on screen that is not true.
    const rust = readFileSync(resolve(__dirname, "../../src-tauri/src/workspace.rs"), "utf8");
    const declared = rust.match(/pub const MAX_MEMORY: usize = ([\d_]+)/);
    expect(declared, "MAX_MEMORY has been renamed or moved out of workspace.rs").not.toBeNull();
    expect(Number(declared![1]!.replaceAll("_", ""))).toBe(CAP);
  });
});
