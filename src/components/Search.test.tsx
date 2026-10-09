import { createEvent, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, SearchHits } from "../lib/types";
import { aGroup } from "../test-fixtures";

/**
 * The palette, over a mocked store.
 *
 * What is worth testing here is not the markup: it is that the two halves of
 * the search behave as one control. The local half has to answer on the
 * keystroke, the stored half has to arrive later without overwriting a newer
 * query, and choosing a row has to do the one thing that row says it does.
 */

const GROUP = "00000000-0000-4000-8000-000000000001";
const NOTHING: SearchHits = { messages: [], files: [], links: [], routines: [], artifacts: [] };

const search = vi.fn<(query: string, limit?: number) => Promise<SearchHits>>(async () => NOTHING);
const openExternal = vi.fn<(url: string) => Promise<void>>(async () => {});

vi.mock("../lib/ipc", () => ({
  api: {
    search: (query: string, limit?: number) => search(query, limit),
    listAgents: async () => [],
    listGroups: async () => [],
    channelMessages: async () => [],
    conversationFlow: async () => [],
  },
  openExternal: (url: string) => openExternal(url),
}));

const { Search } = await import("./Search");
const { useStore } = await import("../lib/store");

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
    hasTerminal: false,
    harness: "pi",
    gate: "open",
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

const handlers = {
  onClose: vi.fn(),
  onEditAgent: vi.fn(),
  onEditGroup: vi.fn(),
  onNewAgent: vi.fn(),
  onNewGroup: vi.fn(),
  onOpenCafeteria: vi.fn(),
  onOpenSettings: vi.fn(),
};

function open(agents: AgentCard[] = [agent("Manager"), agent("Chef")]) {
  useStore.setState({
    agents,
    groups: [aGroup({ id: GROUP, name: "Kitchen", agentCount: agents.length })],
    lastActive: {},
    selected: null,
    messages: {},
    focused: null,
  });
  return render(<Search {...handlers} />);
}

function type(value: string) {
  fireEvent.change(screen.getByLabelText(/search the workspace/i), { target: { value } });
}

/**
 * A key press from the search input, carrying the composition flag a browser
 * puts on the native event.
 *
 * The window listener reads a DOM `KeyboardEvent`, so the flag has to be set
 * on the event itself rather than on a React synthetic wrapper. jsdom has no
 * input method, so this is the only way to reproduce what an IME delivers:
 * a real `keydown` whose native `isComposing` is true.
 */
function press(key: string, options: { isComposing?: boolean; shiftKey?: boolean } = {}) {
  const input = screen.getByLabelText(/search the workspace/i);
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

/** The title of the row the cursor is on, read from the rendered list. */
const selectedTitle = () =>
  document.querySelector("[data-selected='true'] .palette__title")?.textContent;

/** The label of the scope tab currently marked selected. */
const activeScope = () => screen.getByRole("tab", { selected: true }).textContent;

beforeEach(() => {
  vi.clearAllMocks();
  search.mockResolvedValue(NOTHING);
});

describe("Search", () => {
  it("narrows as the operator types, without waiting for the store", async () => {
    // Agents are already in hand, so the list must move on the keystroke
    // itself. A palette that pauses for a round trip to filter two names is
    // slower than the rail it replaced.
    open();
    expect(screen.getByText("Manager")).toBeTruthy();
    expect(screen.getByText("Chef")).toBeTruthy();

    type("chef");
    expect(screen.queryByText("Manager")).toBeNull();
    expect(screen.getByText("Chef")).toBeTruthy();
  });

  it("asks the store about the query, once the typing has settled", async () => {
    open();
    type("bud");
    type("budget");
    await waitFor(() => expect(search).toHaveBeenCalledWith("budget", expect.any(Number)));
    // Both keystrokes inside the window are one query, not two.
    expect(search.mock.calls.filter(([q]) => q === "bud")).toHaveLength(0);
  });

  it("ignores an answer to a query the operator has moved past", async () => {
    // The reply to "b" can land after the reply to "budget". Rendering it
    // would show results for something nobody is looking at any more.
    const stale: SearchHits = {
      ...NOTHING,
      links: [
        { url: "https://stale.example", messageId: "m0", channelId: "id-Chef", createdAt: 1 },
      ],
    };
    const fresh: SearchHits = {
      ...NOTHING,
      links: [
        { url: "https://fresh.example", messageId: "m1", channelId: "id-Chef", createdAt: 2 },
      ],
    };

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
    await waitFor(() => expect(screen.getByText("fresh.example")).toBeTruthy());

    releaseStale(stale);
    await waitFor(() => expect(screen.queryByText("stale.example")).toBeNull());
    expect(screen.getByText("fresh.example")).toBeTruthy();
  });

  it("shows only one kind once a tab is chosen", async () => {
    search.mockResolvedValue({
      ...NOTHING,
      files: [
        {
          file: { digest: "aaa", name: "budget.pdf", mime: "application/pdf", bytes: 2048 },
          messageId: "m1",
          channelId: "id-Chef",
          from: { kind: "human" },
          createdAt: 2,
        },
      ],
    });
    open();
    await waitFor(() => expect(screen.getByText("budget.pdf")).toBeTruthy());

    fireEvent.click(screen.getByRole("tab", { name: "Files" }));
    expect(screen.getByText("budget.pdf")).toBeTruthy();
    expect(screen.queryByText("Manager")).toBeNull();

    fireEvent.click(screen.getByRole("tab", { name: "Agents" }));
    expect(screen.getByText("Manager")).toBeTruthy();
    expect(screen.queryByText("budget.pdf")).toBeNull();
  });

  it("opens the selected result on Enter and closes", async () => {
    open();
    type("chef");
    fireEvent.keyDown(window, { key: "Enter" });

    await waitFor(() => expect(useStore.getState().selected).toBe("id-Chef"));
    expect(handlers.onClose).toHaveBeenCalled();
  });

  it("moves the selection with the arrow keys", async () => {
    open();
    const selected = () =>
      document.querySelector("[data-selected='true'] .palette__title")?.textContent;
    // Read off the list rather than assumed: what order the rows come in is
    // the ranking's business, and it is tested where the ranking is.
    const rows = [...document.querySelectorAll(".palette__title")].map((n) => n.textContent);

    expect(selected()).toBe(rows[0]);
    fireEvent.keyDown(window, { key: "ArrowDown" });
    expect(selected()).toBe(rows[1]);
    fireEvent.keyDown(window, { key: "ArrowUp" });
    expect(selected()).toBe(rows[0]);
  });

  it("does not run off the end of the list", async () => {
    // Enter on a cursor past the last row would open nothing, or worse, open
    // whatever ends up at that index when the next answer arrives.
    open([agent("Manager")]);
    fireEvent.click(screen.getByRole("tab", { name: "Agents" }));
    for (let i = 0; i < 5; i++) fireEvent.keyDown(window, { key: "ArrowDown" });

    expect(document.querySelector("[data-selected='true'] .palette__title")?.textContent).toBe(
      "Manager",
    );
  });

  it("closes on Escape without opening anything", async () => {
    open();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(handlers.onClose).toHaveBeenCalled();
    expect(useStore.getState().selected).toBeNull();
  });

  it("opens a link in the operating system browser, never in the webview", async () => {
    // Following one inside the webview navigates away from the app with no
    // way back.
    search.mockResolvedValue({
      ...NOTHING,
      links: [
        { url: "https://example.com/report", messageId: "m1", channelId: "id-Chef", createdAt: 2 },
      ],
    });
    open();
    await waitFor(() => expect(screen.getByText("example.com/report")).toBeTruthy());

    fireEvent.click(screen.getByText("example.com/report"));
    expect(openExternal).toHaveBeenCalledWith("https://example.com/report");
  });

  it("hands an agent's settings to the editor rather than opening the channel", async () => {
    open();
    type("Manager settings");
    fireEvent.keyDown(window, { key: "Enter" });

    expect(handlers.onEditAgent).toHaveBeenCalledWith(expect.objectContaining({ name: "Manager" }));
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
    // Marked, so the transcript can scroll to it and say which one it was.
    expect(useStore.getState().focused).toBe("m1");
  });

  it("says so when nothing matches, rather than looking broken", async () => {
    open();
    type("nothing here by that name");
    expect(screen.getByText(/nothing matching/i)).toBeTruthy();
  });

  describe("while an input method is composing", () => {
    it("does not open a result or close the palette on Enter", async () => {
      // An IME uses Enter to accept a candidate, not to submit. The palette
      // must leave that keystroke alone or it would open something mid-word.
      open();
      type("chef");

      const event = press("Enter", { isComposing: true });

      expect(event.defaultPrevented).toBe(false);
      expect(useStore.getState().selected).toBeNull();
      expect(handlers.onClose).not.toHaveBeenCalled();
    });

    it("does not move the selection down with ArrowDown", async () => {
      open();
      const before = selectedTitle();

      const event = press("ArrowDown", { isComposing: true });

      expect(event.defaultPrevented).toBe(false);
      expect(selectedTitle()).toBe(before);
    });

    it("does not move the selection up with ArrowUp", async () => {
      // Move off the first row first: a composing ArrowUp that only failed to
      // move because it was already at the top would prove nothing.
      open();
      press("ArrowDown");
      const second = selectedTitle();
      expect(second).not.toBe(selectedTitleAtStart());

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
      // The cursor is not reset either: a scope change clears it.
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
      expect(handlers.onClose).not.toHaveBeenCalled();
      expect(useStore.getState().selected).toBeNull();
    });

    it("recovers on the ordinary key that follows composition", async () => {
      // The guard must be a pass-through, not a one-way latch: once the IME
      // is done, the very next ordinary Enter has to work.
      open();
      type("chef");
      press("Enter", { isComposing: true });
      expect(handlers.onClose).not.toHaveBeenCalled();

      press("Enter");

      await waitFor(() => expect(useStore.getState().selected).toBe("id-Chef"));
      expect(handlers.onClose).toHaveBeenCalled();
    });
  });

  describe("ordinary keyboard input", () => {
    it("still moves down and up one row and stops at the bounds", async () => {
      open();
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

    it("cycles the scope forward, wrapping Actions back to All", () => {
      open();
      expect(activeScope()).toBe("All");
      for (const label of [
        "Messages",
        "Agents",
        "Groups",
        "Files",
        "Links",
        "Routines",
        "Artifacts",
        "Actions",
        "All",
      ]) {
        const event = press("Tab");
        expect(event.defaultPrevented).toBe(true);
        expect(activeScope()).toBe(label);
      }
    });

    it("cycles the scope backward, wrapping Messages back to All", () => {
      open();
      const event = press("Tab", { shiftKey: true });
      expect(event.defaultPrevented).toBe(true);
      expect(activeScope()).toBe("Actions");
      for (const label of [
        "Artifacts",
        "Routines",
        "Links",
        "Files",
        "Groups",
        "Agents",
        "Messages",
        "All",
      ]) {
        press("Tab", { shiftKey: true });
        expect(activeScope()).toBe(label);
      }
    });

    it("still filters on typed text after composing navigation", () => {
      open();
      press("ArrowDown", { isComposing: true });
      type("chef");
      expect(screen.queryByText("Manager")).toBeNull();
      expect(screen.getByText("Chef")).toBeTruthy();
    });
  });
});

/** The title of the first row, read without touching the cursor. */
function selectedTitleAtStart() {
  return document.querySelector(".palette__title")?.textContent;
}
