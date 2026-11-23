import { act, createEvent, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, SearchHits } from "../lib/types";
import { searchRequest } from "../lib/useSearchHits";
import { aGroup } from "../test-fixtures";

const GROUP = "00000000-0000-4000-8000-000000000001";
const NOTHING: SearchHits = { messages: [], files: [], links: [], routines: [], artifacts: [] };

const search = vi.fn<(query: string, limit?: number) => Promise<SearchHits>>(async () => NOTHING);

vi.mock("../lib/ipc", () => ({
  api: {
    search: (query: string, limit?: number) => search(query, limit),
    listAgents: async () => [],
    listGroups: async () => [],
    channelMessages: async () => [],
    conversationFlow: async () => [],
  },
}));

const { Search } = await import("./Search");
const { useStore } = await import("../lib/store");
const originalShowFailure = useStore.getState().showFailure;

function agent(name: string, extra: Partial<AgentCard> = {}): AgentCard {
  return {
    id: `id-${name}`,
    groupId: GROUP,
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "test/model",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 1,
    updatedAt: 1,
    discardedAt: null,
    ...extra,
  };
}

const onClose = vi.fn();

function open() {
  useStore.setState({
    agents: [agent("Manager"), agent("Chef")],
    groups: [aGroup({ id: GROUP, name: "Kitchen", agentCount: 2 })],
    lastActive: {},
    selected: null,
    messages: {},
    focused: null,
  });
  return render(<Search onClose={onClose} />);
}

function fileHits(...names: string[]): SearchHits {
  return {
    ...NOTHING,
    files: names.map((name, index) => ({
      file: { digest: name, name, mime: "text/plain", bytes: 2048 },
      messageId: `m${index + 1}`,
      channelId: "id-Chef",
      from: { kind: "human" },
      createdAt: index + 1,
    })),
  };
}

async function openWithFiles(...names: string[]) {
  search.mockResolvedValue(fileHits(...names));
  open();
  await screen.findByText(names[0] ?? "");
}

function type(value: string) {
  fireEvent.change(screen.getByLabelText(/search guaca/i), { target: { value } });
}

function press(key: string, options: { isComposing?: boolean; shiftKey?: boolean } = {}) {
  const input = screen.getByLabelText(/search guaca/i);
  const event = createEvent.keyDown(input, {
    key,
    isComposing: options.isComposing ?? false,
    shiftKey: options.shiftKey ?? false,
    bubbles: true,
    cancelable: true,
  });
  fireEvent(input, event);
  return event;
}

const selectedTitle = () =>
  document.querySelector("[data-selected='true'] .palette__title")?.textContent;

const activeScope = () => screen.getByRole("tab", { selected: true }).textContent;

beforeEach(() => {
  vi.clearAllMocks();
  search.mockResolvedValue(NOTHING);
  useStore.setState({ showFailure: originalShowFailure });
});

describe("Search", () => {
  it("limits Find to messages and files and never offers an app action", () => {
    open();
    expect(screen.getAllByRole("tab").map((tab) => tab.textContent)).toEqual([
      "All",
      "Messages",
      "Files",
    ]);
    type("new agent");
    press("Enter");
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.queryByText("New agent")).toBeNull();
  });

  it("transitions request ownership only for the current generation", () => {
    const inactive = { phase: "inactive" } as const;
    const debounce = searchRequest(inactive, { type: "begin", generation: 1 });
    expect(debounce).toEqual({ phase: "debouncing", generation: 1 });
    expect(searchRequest(debounce, { type: "finish", generation: 1 })).toBe(debounce);
    expect(searchRequest(debounce, { type: "start", generation: 2 })).toBe(debounce);
    const searching = searchRequest(debounce, { type: "start", generation: 1 });
    expect(searching).toEqual({ phase: "searching", generation: 1 });
    expect(searchRequest(searching, { type: "start", generation: 1 })).toBe(searching);
    const settled = searchRequest(searching, { type: "finish", generation: 1 });
    expect(settled).toEqual({ phase: "settled", generation: 1 });
    expect(searchRequest(settled, { type: "finish", generation: 1 })).toBe(settled);
    expect(searchRequest(settled, { type: "cancel", generation: 1 })).toEqual(inactive);
    expect(searchRequest(inactive, { type: "cancel", generation: 1 })).toBe(inactive);
    expect(searchRequest(searching, { type: "begin", generation: 2 })).toEqual({
      phase: "debouncing",
      generation: 2,
    });
    expect(searchRequest(searching, { type: "cancel", generation: 2 })).toBe(searching);
  });

  it("debounces and reports a current failure", async () => {
    vi.useFakeTimers();
    try {
      const failure = vi.fn();
      const error = new Error("current");
      useStore.setState({ showFailure: failure });
      search.mockRejectedValueOnce(error);
      open();
      type("current");
      await act(async () => vi.advanceTimersByTime(119));
      expect(search).not.toHaveBeenCalled();
      await act(async () => vi.advanceTimersByTime(1));
      expect(search).toHaveBeenCalledTimes(1);
      expect(failure).toHaveBeenCalledExactlyOnceWith(error);
      expect(screen.getByText("Could not search messages and files.")).toBeTruthy();
    } finally {
      vi.useRealTimers();
    }
  });
  it("ignores a failure from an older query after the current query succeeds", async () => {
    vi.useFakeTimers();
    try {
      const failure = vi.fn();
      useStore.setState({ showFailure: failure });
      let rejectOld!: (error: Error) => void;
      search.mockImplementationOnce(
        () =>
          new Promise((_, reject) => {
            rejectOld = reject;
          }),
      );
      search.mockResolvedValueOnce(NOTHING);
      open();
      type("old");
      await act(async () => vi.advanceTimersByTime(120));
      type("new");
      await act(async () => vi.advanceTimersByTime(120));
      expect(search.mock.calls.map(([query]) => query)).toEqual(["old", "new"]);
      await act(async () => rejectOld(new Error("obsolete")));
      expect(failure).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("ignores a failure after the palette unmounts", async () => {
    vi.useFakeTimers();
    try {
      const failure = vi.fn();
      useStore.setState({ showFailure: failure });
      let reject!: (error: Error) => void;
      search.mockImplementationOnce(
        () =>
          new Promise((_, fail) => {
            reject = fail;
          }),
      );
      const view = open();
      await act(async () => vi.advanceTimersByTime(120));
      view.unmount();
      await act(async () => reject(new Error("closed")));
      expect(failure).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
  it("does not open a previous hit while a different query is pending", async () => {
    await openWithFiles("previous.txt");
    type("new query");
    expect(screen.queryByText("previous.txt")).toBeNull();
    press("Enter");
    expect(onClose).not.toHaveBeenCalled();
    expect(useStore.getState().focused).toBeNull();
  });

  it("asks the store about the query, once the typing has settled", async () => {
    open();
    type("bud");
    type("budget");
    await waitFor(() => expect(search).toHaveBeenCalledWith("budget", expect.any(Number)));
    expect(search.mock.calls.filter(([q]) => q === "bud")).toHaveLength(0);
  });

  it("ignores an answer to a query the operator has moved past", async () => {
    const stale = fileHits("stale.txt");
    const fresh = fileHits("fresh.txt");

    let releaseStale: (hits: SearchHits) => void = () => {};
    search.mockImplementationOnce(
      () =>
        new Promise<SearchHits>((resolve) => {
          releaseStale = resolve;
        }),
    );
    search.mockImplementationOnce(async () => fresh);

    open();
    type("b");
    await waitFor(() => expect(search).toHaveBeenCalledTimes(1));
    type("budget");
    await waitFor(() => expect(screen.getByText("fresh.txt")).toBeTruthy());

    releaseStale(stale);
    await waitFor(() => expect(screen.queryByText("stale.txt")).toBeNull());
    expect(screen.getByText("fresh.txt")).toBeTruthy();
  });

  it("shows files in All and Files but not Messages", async () => {
    await openWithFiles("budget.pdf");
    fireEvent.click(screen.getByRole("tab", { name: "Files" }));
    expect(screen.getByText("budget.pdf")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "Messages" }));
    expect(screen.queryByText("budget.pdf")).toBeNull();
    fireEvent.click(screen.getByRole("tab", { name: "All" }));
    expect(screen.getByText("budget.pdf")).toBeTruthy();
  });

  it("opens the selected file's message on Enter and closes", async () => {
    await openWithFiles("budget.pdf");
    press("Enter");
    await waitFor(() => expect(useStore.getState().selected).toBe("id-Chef"));
    expect(useStore.getState().focused).toBe("m1");
    expect(onClose).toHaveBeenCalled();
  });

  it("moves the selection with the arrow keys", async () => {
    await openWithFiles("budget.pdf", "invoice.txt");
    const selected = () =>
      document.querySelector("[data-selected='true'] .palette__title")?.textContent;
    const rows = [...document.querySelectorAll(".palette__title")].map((n) => n.textContent);

    expect(selected()).toBe(rows[0]);
    fireEvent.keyDown(window, { key: "ArrowDown" });
    expect(selected()).toBe(rows[1]);
    fireEvent.keyDown(window, { key: "ArrowUp" });
    expect(selected()).toBe(rows[0]);
  });

  it("does not run off the end of the list", async () => {
    await openWithFiles("budget.pdf");
    for (let i = 0; i < 5; i++) press("ArrowDown");
    expect(selectedTitle()).toBe("budget.pdf");
  });

  it("closes on Escape without opening anything", async () => {
    open();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
    expect(useStore.getState().selected).toBeNull();
  });

  it("opens a message in the channel it was written in", async () => {
    search.mockResolvedValue({
      ...NOTHING,
      messages: [
        {
          id: "m1",
          channelId: "id-Chef",
          from: { kind: "human" },
          to: { kind: "agent", id: "id-Chef" },
          excerpt: "the budget is signed off",
          createdAt: 2,
        },
      ],
    });
    open();
    await waitFor(() => expect(screen.getByText("the budget is signed off")).toBeTruthy());

    fireEvent.click(screen.getByText("the budget is signed off"));
    await waitFor(() => expect(useStore.getState().selected).toBe("id-Chef"));
    expect(useStore.getState().focused).toBe("m1");
  });

  it("says so when nothing matches, rather than looking broken", async () => {
    open();
    type("nothing here by that name");
    expect(screen.getByText("Searching…")).toBeTruthy();
    expect(await screen.findByText(/nothing matching/i)).toBeTruthy();
  });

  describe("while an input method is composing", () => {
    it("does not open a result or close the palette on Enter", async () => {
      await openWithFiles("budget.pdf");

      const event = press("Enter", { isComposing: true });

      expect(event.defaultPrevented).toBe(false);
      expect(useStore.getState().selected).toBeNull();
      expect(onClose).not.toHaveBeenCalled();
    });

    it("does not move the selection down with ArrowDown", async () => {
      open();
      const before = selectedTitle();

      const event = press("ArrowDown", { isComposing: true });

      expect(event.defaultPrevented).toBe(false);
      expect(selectedTitle()).toBe(before);
    });

    it("does not move the selection up with ArrowUp", async () => {
      await openWithFiles("budget.pdf", "invoice.txt");
      const first = selectedTitle();
      press("ArrowDown");
      const second = selectedTitle();
      expect(second).not.toBe(first);

      const event = press("ArrowUp", { isComposing: true });

      expect(event.defaultPrevented).toBe(false);
      expect(selectedTitle()).toBe(second);
    });

    it("does not cycle the scope forward with Tab", async () => {
      open();
      press("ArrowDown");
      const cursor = selectedTitle();

      const event = press("Tab", { isComposing: true });

      expect(event.defaultPrevented).toBe(false);
      expect(activeScope()).toBe("All");
      expect(selectedTitle()).toBe(cursor);
    });

    it("does not cycle the scope backward with Shift+Tab", async () => {
      open();
      press("ArrowDown");
      const cursor = selectedTitle();

      const event = press("Tab", { isComposing: true, shiftKey: true });

      expect(event.defaultPrevented).toBe(false);
      expect(activeScope()).toBe("All");
      expect(selectedTitle()).toBe(cursor);
    });

    it("does not close on Escape", async () => {
      open();
      type("chef");

      const event = press("Escape", { isComposing: true });

      expect(event.defaultPrevented).toBe(false);
      expect(onClose).not.toHaveBeenCalled();
      expect(useStore.getState().selected).toBeNull();
    });

    it("recovers on the ordinary key that follows composition", async () => {
      await openWithFiles("budget.pdf");
      press("Enter", { isComposing: true });
      expect(onClose).not.toHaveBeenCalled();

      press("Enter");

      await waitFor(() => expect(useStore.getState().selected).toBe("id-Chef"));
      expect(onClose).toHaveBeenCalled();
    });
  });

  describe("ordinary keyboard input", () => {
    it("still moves down and up one row and stops at the bounds", async () => {
      await openWithFiles("budget.pdf", "invoice.txt");
      const rows = [...document.querySelectorAll(".palette__title")].map((n) => n.textContent);

      const down = press("ArrowDown");
      expect(down.defaultPrevented).toBe(true);
      expect(selectedTitle()).toBe(rows[1]);

      const up = press("ArrowUp");
      expect(up.defaultPrevented).toBe(true);
      expect(selectedTitle()).toBe(rows[0]);

      press("ArrowUp");
      expect(selectedTitle()).toBe(rows[0]);
    });

    it("moves on a key pressed in the same tick the hits are drawn", async () => {
      search.mockResolvedValue(fileHits("budget.pdf", "invoice.txt"));
      open();
      const drawn = new Promise<{ second: unknown; moved: unknown }>((resolve) => {
        const watch = new MutationObserver(() => {
          const rows = document.querySelectorAll(".palette__title");
          if (rows.length < 2) return;
          watch.disconnect();
          const second = rows[1]?.textContent;
          press("ArrowDown");
          resolve({ second, moved: selectedTitle() });
        });
        watch.observe(document.body, { childList: true, subtree: true });
      });

      const { second, moved } = await waitFor(() => drawn);

      expect(moved).toBe(second);
    });

    it("cycles the scope forward, wrapping Files back to All", () => {
      open();
      expect(activeScope()).toBe("All");
      for (const label of ["Messages", "Files", "All"]) {
        const event = press("Tab");
        expect(event.defaultPrevented).toBe(true);
        expect(activeScope()).toBe(label);
      }
    });

    it("cycles the scope backward, wrapping All to Files", () => {
      open();
      for (const label of ["Files", "Messages", "All"]) {
        const event = press("Tab", { shiftKey: true });
        expect(event.defaultPrevented).toBe(true);
        expect(activeScope()).toBe(label);
      }
    });

    it("still searches typed text after composing navigation", async () => {
      open();
      press("ArrowDown", { isComposing: true });
      search.mockResolvedValue(fileHits("chef.txt"));
      type("chef");
      expect(await screen.findByText("chef.txt")).toBeTruthy();
    });
  });
});
