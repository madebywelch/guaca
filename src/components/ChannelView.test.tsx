import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { Activity, AgentCard, Envelope, MessageId, Part } from "../lib/types";
import { ChannelView } from "./ChannelView";

/**
 * What a channel shows, and what it keeps one click away.
 *
 * The report behind all of this: a channel that reads as a conversation with
 * one agent, until that agent starts working, at which point the operator's
 * own thread is pushed off the screen by traffic addressed to somebody else.
 *
 * Arriving from search is here too, because the two features meet: search
 * finds a message by its text and hands back an id, and a message between two
 * agents no longer has a row of its own to be put next to.
 */

const pairMessages = vi.fn<() => Promise<Envelope[]>>(async () => []);

vi.mock("../lib/ipc", () => ({
  api: {
    channelMessages: async () => [],
    conversationFlow: async () => [],
    clearChannel: async () => 0,
    sendMessage: async () => "run",
    agentComputer: async () => null,
    pairMessages: () => pairMessages(),
  },
  onFileDrop: async () => () => {},
  openExternal: async () => {},
}));

const MANAGER = "00000000-0000-4000-8000-0000000000a1";
const CHEF = "00000000-0000-4000-8000-0000000000a2";

function card(id: string, name: string): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-0000000000b1",
    sandboxId: null,
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
  };
}

let clock = 1_700_000_000_000;

function envelope(overrides: Partial<Envelope>): Envelope {
  clock += 1_000;
  return {
    id: `m${clock}`,
    runId: "r1",
    channelId: MANAGER,
    from: { kind: "human" },
    to: { kind: "agent", id: MANAGER },
    parts: [{ type: "text", text: "hello" }],
    trust: "operator",
    hop: 0,
    expectsReply: true,
    intent: "work",
    cause: null,
    createdAt: clock,
    ...overrides,
  };
}

function fromPeer(text: string): Envelope {
  return envelope({
    from: { kind: "agent", id: CHEF },
    to: { kind: "agent", id: MANAGER },
    parts: [{ type: "text", text }],
  });
}

function record(part: Part): Envelope {
  return envelope({
    from: { kind: "agent", id: MANAGER },
    to: { kind: "system" },
    trust: "system",
    parts: [part],
  });
}

function open(messages: Envelope[]) {
  useStore.setState({
    agents: [card(MANAGER, "Manager"), card(CHEF, "Chef")],
    messages: { [MANAGER]: messages },
    streams: {},
    activity: {},
    lastActive: {},
    banner: null,
  });
  return render(<ChannelView channel={MANAGER} onOpenMenu={() => {}} />);
}

beforeEach(() => {
  pairMessages.mockClear();
  pairMessages.mockResolvedValue([]);
  // A mark left by the last test would have this one jumping somewhere.
  useStore.setState({ focused: null });
});

describe("peer traffic in a channel", () => {
  it("shows one line for a burst, not one per message", () => {
    open([
      envelope({}),
      fromPeer("a long report nobody asked to read here"),
      fromPeer("and another"),
    ]);

    expect(screen.getByRole("button", { name: /2 messages with Chef/ })).toBeTruthy();
    expect(screen.queryByText("a long report nobody asked to read here")).toBeNull();
  });

  it("opens the pair's own thread when the line is clicked", async () => {
    pairMessages.mockResolvedValue([
      envelope({
        from: { kind: "agent", id: MANAGER },
        to: { kind: "agent", id: CHEF },
        parts: [{ type: "text", text: "what is the status" }],
      }),
      fromPeer("all clear"),
    ]);

    open([fromPeer("all clear")]);
    fireEvent.click(screen.getByRole("button", { name: /Message from Chef/ }));

    // Both sides, in full. This is what the row exists to lead to.
    expect(await screen.findByText("what is the status")).toBeTruthy();
    expect(screen.getByText("all clear")).toBeTruthy();
    expect(screen.getByText(/view-only/)).toBeTruthy();
  });

  it("does not offer to send anything from inside that thread", async () => {
    // Neither of those two agents is addressable from here, and a composer
    // that refuses is worse than saying so.
    open([fromPeer("all clear")]);
    fireEvent.click(screen.getByRole("button", { name: /Message from Chef/ }));

    await screen.findByText(/view-only/);
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("comes back to the channel when the thread is closed", async () => {
    open([
      envelope({ parts: [{ type: "text", text: "the operator's own message" }] }),
      fromPeer("hi"),
    ]);
    fireEvent.click(screen.getByRole("button", { name: /Message from Chef/ }));
    await screen.findByText(/view-only/);

    fireEvent.click(screen.getByRole("button", { name: "Close chat" }));
    await waitFor(() => expect(screen.getByText("the operator's own message")).toBeTruthy());
  });

  it("keeps a refused send out of the burst, with its reason on the line", () => {
    // A stop is not part of the conversation, and folding it into a count
    // would report a message that never arrived as one that did.
    open([
      fromPeer("hi"),
      record({
        type: "toolCall",
        name: "send_message",
        arguments: { to: ["Chef"], text: "the same thing again" },
        outcome: {
          status: "refused",
          reason: "Refused: you already sent Chef this exact message in this run. Move on.",
        },
      }),
    ]);

    expect(screen.getByText(/Not delivered to Chef/)).toBeTruthy();
    expect(screen.getByText("you already sent Chef this exact message in this run")).toBeTruthy();
  });
});

describe("who said what", () => {
  it("does not write the agent's name over every message it sent", () => {
    // A channel has two participants: the one named at the top of the pane and
    // the operator reading it. A name and a clock over each of four replies
    // written in the same minute is eight lines carrying two facts.
    open([
      envelope({ parts: [{ type: "text", text: "what is the status" }] }),
      envelope({
        from: { kind: "agent", id: MANAGER },
        to: { kind: "human" },
        parts: [{ type: "text", text: "all clear" }],
      }),
    ]);

    expect(screen.getByText("all clear")).toBeTruthy();
    // Once, at the top of the pane, and nowhere in the transcript.
    expect(screen.getAllByText("Manager")).toHaveLength(1);
    expect(screen.queryByText("You")).toBeNull();
  });

  it("says when the conversation picked up again, where it did", () => {
    const morning = envelope({ parts: [{ type: "text", text: "first thing" }] });
    const afternoon = envelope({
      createdAt: morning.createdAt + 5 * 60 * 60 * 1000,
      parts: [{ type: "text", text: "back again" }],
    });

    const { container } = open([morning, afternoon]);
    const line = container.querySelector(".when-row time");
    expect(line?.getAttribute("datetime")).toBe(new Date(afternoon.createdAt).toISOString());
    // One line for the gap, not one clock per message.
    expect(container.querySelectorAll(".when-row")).toHaveLength(1);
  });
});

describe("while an agent is working", () => {
  /** What the runtime tells the UI as a turn moves through its states. */
  const doing = (state: Activity) =>
    act(() => useStore.setState({ activity: { [MANAGER]: state } }));

  it("says so above the composer, naming it", () => {
    // The gap between sending and the first token is the moment the app looks
    // broken, and a turn can spend several model calls before writing a word.
    open([envelope({})]);
    expect(screen.queryByText("Manager is working")).toBeNull();

    doing({ state: "thinking" });
    expect(screen.getByText("Manager is working")).toBeTruthy();

    doing({ state: "idle" });
    expect(screen.queryByText("Manager is working")).toBeNull();
  });

  it("counts unread work as working, and waiting on a person as not", () => {
    open([envelope({})]);

    doing({ state: "queued", depth: 2 });
    expect(screen.getByText("Manager is working")).toBeTruthy();

    // Parked on a permission request. The request itself is in this channel
    // saying what it needs, and it needs the operator, not time.
    doing({ state: "awaitingApproval" });
    expect(screen.queryByText("Manager is working")).toBeNull();
  });
});

describe("a message arrived at from search", () => {
  it("is marked and brought into view", async () => {
    const scrolled = vi.spyOn(Element.prototype, "scrollIntoView");
    const rows = Array.from({ length: 12 }, () => envelope({}));
    useStore.setState({ focused: rows[0]!.id });

    const { container } = open(rows);

    await waitFor(() => {
      expect(
        container.querySelector(`[data-message="${rows[0]!.id}"][data-found="true"]`),
      ).toBeTruthy();
    });
    expect(scrolled).toHaveBeenCalled();
    // One row, not the whole transcript lit up.
    expect(container.querySelectorAll("[data-found='true']")).toHaveLength(1);
    scrolled.mockRestore();
  });

  it("stops being marked once it has been shown", async () => {
    // A mark that outlives the visit is still there next time the channel is
    // opened, saying nothing about anything.
    vi.useFakeTimers();
    try {
      const rows = Array.from({ length: 12 }, () => envelope({}));
      useStore.setState({ focused: rows[0]!.id });
      const { container } = open(rows);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(5_000);
      });
      expect(useStore.getState().focused).toBeNull();
      expect(container.querySelector("[data-found='true']")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("leaves the transcript alone when the message is not in the window", async () => {
    // The window is bounded, so a hit older than it opens the channel at the
    // newest end. That is a partial jump, not a broken transcript.
    const missing = "00000000-0000-4000-8000-999999999999" as MessageId;
    useStore.setState({ focused: missing });
    const { container } = open(Array.from({ length: 12 }, () => envelope({})));

    await waitFor(() => expect(container.querySelectorAll("[data-message]")).toHaveLength(12));
    expect(container.querySelector("[data-found='true']")).toBeNull();
    // Still held, so nothing has quietly decided the jump succeeded.
    expect(useStore.getState().focused).toBe(missing);
  });

  it("lands on the row that swallowed a message between two agents", async () => {
    // Search matches any message with the text in it, and one between two
    // agents has no row of its own here any more. The row that collapsed it is
    // where it is in this channel, and opening the thread is how it is read.
    //
    // The wanted one is second in the burst deliberately: a row stands for
    // every message it swallowed, not just the one that opened it.
    const scrolled = vi.spyOn(Element.prototype, "scrollIntoView");
    const first = fromPeer("something else entirely");
    const wanted = fromPeer("the detail somebody went looking for");
    useStore.setState({ focused: wanted.id });

    const { container } = open([envelope({}), first, wanted]);

    await waitFor(() => {
      const marked = container.querySelector("[data-found='true']");
      expect(marked?.textContent).toContain("2 messages with Chef");
    });
    expect(scrolled).toHaveBeenCalled();
    scrolled.mockRestore();
  });
});
