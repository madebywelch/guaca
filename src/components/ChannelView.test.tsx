import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ACTIVITY_CHANNEL, useStore } from "../lib/store";
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
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
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
    reasoning: {},
    trail: {},
    activity: {},
    lastActive: {},
    banner: null,
  });
  return render(
    <ChannelView
      channel={MANAGER}
      onOpenMenu={() => {}}
      onNewAgent={() => {}}
      onNewGroup={() => {}}
    />,
  );
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

  it("shows what it is thinking, and stops when the thought is dropped", () => {
    // The complaint this answers: a pulsing avatar says a turn is alive and
    // nothing says what it is doing, through a wait that can run to ten
    // minutes.
    open([envelope({})]);
    doing({ state: "thinking" });

    act(() => useStore.setState({ reasoning: { [MANAGER]: "**Checking**\n\nthe totals agree." } }));
    expect(screen.getByText("Checking")).toBeTruthy();
    expect(screen.getByText("the totals agree.")).toBeTruthy();
    expect(screen.queryByText("Manager is working")).toBeNull();
    // And it stops being a live region while it holds a line that moves under
    // it, or a screen reader reads out every half sentence of it over whatever
    // else it was saying. Asked of this line rather than of the pane: an
    // arriving message is announced by a region of its own, which is the one
    // thing here that is meant to be heard.
    expect(document.querySelector(".working")?.getAttribute("role")).toBeNull();

    // The runtime clears it when the stream ends, and the line goes back to
    // saying only that the turn is still going.
    act(() => useStore.setState({ reasoning: {} }));
    const note = document.querySelector(".working");
    expect(note?.getAttribute("role")).toBe("status");
    expect(note?.textContent).toContain("Manager is working");
  });

  it("draws the heading alone until a sentence under it has finished", () => {
    // A sentence being typed at streaming speed cannot be read, and replacing
    // the line with half of the next one sixty times a second is the flicker
    // the heading exists to stop.
    open([envelope({})]);
    doing({ state: "thinking" });
    act(() => useStore.setState({ reasoning: { [MANAGER]: "**Checking**\n\nthe totals do" } }));

    expect(screen.getByText("Checking")).toBeTruthy();
    expect(screen.queryByText("the totals do")).toBeNull();
  });

  it("opens the whole of the working, and closes it when the turn ends", () => {
    // A peek is enough right up until the wait runs to ten minutes, when the
    // question stops being "is it alive" and becomes "is it doing something
    // sensible", which cannot be answered one sentence at a time.
    open([envelope({})]);
    doing({ state: "thinking" });
    const held = "**Checking**\n\nthe totals agree.\n\nSo the third quarter is the one to redo.";
    act(() => useStore.setState({ reasoning: { [MANAGER]: held } }));

    expect(document.querySelector(".thought")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Show what Manager is working through/ }));
    expect(document.querySelector(".thought__text")?.textContent).toBe(held);

    // Asking to watch one turn's working is not a standing decision to watch
    // every turn's.
    doing({ state: "idle" });
    doing({ state: "thinking" });
    expect(document.querySelector(".thought")).toBeNull();
  });

  it("offers nothing to open where the model published nothing", () => {
    // Anthropic's models over OpenRouter publish no working unless it is asked
    // for. A control that opens an empty box is one the operator stops
    // trusting the rest of.
    open([envelope({})]);
    doing({ state: "thinking" });

    expect(screen.queryByRole("button", { name: /working through/ })).toBeNull();
  });

  it("draws the calls the turn has already made, as the transcript will", () => {
    // Until this, a turn's tool calls were invisible for as long as the turn
    // ran: a browsing turn spends most of its twenty-four rounds in the
    // browser and the operator watching had one line of prose.
    open([envelope({})]);
    doing({ state: "thinking" });
    act(() =>
      useStore.setState({
        trail: {
          [MANAGER]: [
            {
              callId: "call_1",
              name: "browse",
              arguments: { action: "open", url: "https://www.cnn.com/world" },
              done: {
                type: "toolCall",
                name: "browse",
                arguments: { action: "open", url: "https://www.cnn.com/world" },
                outcome: { status: "ok", summary: "read cnn.com" },
              },
              startedAt: 0,
            },
          ],
        },
      }),
    );

    expect(screen.getByText("Opened cnn.com")).toBeTruthy();
  });

  it("says what it is waiting on, and how long it has been", () => {
    // While a command runs the model is not thinking, so its last thought is
    // frozen and stale, which is the state that reads as a hang.
    open([envelope({})]);
    doing({ state: "thinking" });
    act(() =>
      useStore.setState({
        reasoning: { [MANAGER]: "I should check the tests." },
        trail: {
          [MANAGER]: [
            {
              callId: "call_1",
              name: "run_command",
              arguments: { command: "npm test" },
              done: null,
              // Half a second past the whole, so the floor is the same number
              // whichever side of a millisecond the render lands on.
              startedAt: Date.now() - 74_500,
            },
          ],
        },
      }),
    );

    expect(screen.getByText("Running a command")).toBeTruthy();
    expect(screen.getByText("1m 14s")).toBeTruthy();
    // The thought is still there to open; it is just not what the line says.
    expect(screen.queryByText("I should check the tests.")).toBeNull();
    expect(screen.getByRole("button", { name: /working through/ })).toBeTruthy();
  });

  it("leaves the line alone for a call that answers immediately", () => {
    // A directory lookup answers in milliseconds. A line that flashed
    // "Checking who is available" for each of them would put back the flicker
    // the sentence rule takes out.
    open([envelope({})]);
    doing({ state: "thinking" });
    act(() =>
      useStore.setState({
        reasoning: { [MANAGER]: "I should see who is here." },
        trail: {
          [MANAGER]: [
            {
              callId: "call_1",
              name: "directory",
              arguments: {},
              done: null,
              startedAt: Date.now(),
            },
          ],
        },
      }),
    );

    expect(screen.queryByText("Checking who is available")).toBeNull();
    expect(screen.getByText("I should see who is here.")).toBeTruthy();
  });

  it("draws another agent's calls in that agent's channel and not this one", () => {
    open([envelope({})]);
    doing({ state: "thinking" });
    act(() =>
      useStore.setState({
        trail: {
          [CHEF]: [
            {
              callId: "call_1",
              name: "run_command",
              arguments: { command: "not this channel" },
              done: {
                type: "toolCall",
                name: "run_command",
                arguments: { command: "not this channel" },
                outcome: { status: "ok", summary: "exit 0" },
              },
              startedAt: 0,
            },
          ],
        },
      }),
    );

    expect(screen.queryByText("Ran a command")).toBeNull();
    expect(screen.getByText("Manager is working")).toBeTruthy();
  });

  it("draws another agent's thinking in that agent's channel and not this one", () => {
    open([envelope({})]);
    doing({ state: "thinking" });
    act(() => useStore.setState({ reasoning: { [CHEF]: "not this channel" } }));

    expect(screen.queryByText("not this channel")).toBeNull();
    expect(screen.getByText("Manager is working")).toBeTruthy();
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

describe("a transcript scrolled up", () => {
  /**
   * jsdom does no layout, so the transcript is given a size here. `scrollTop`
   * clamps and fires a scroll event exactly as a real box does, which is the
   * only thing the transcript has to tell it where the operator is looking.
   */
  function measure(el: HTMLElement, content: number, viewport: number) {
    let top = 0;
    Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => content });
    Object.defineProperty(el, "clientHeight", { configurable: true, get: () => viewport });
    Object.defineProperty(el, "scrollTop", {
      configurable: true,
      get: () => top,
      set: (next: number) => {
        const landed = Math.max(0, Math.min(next, content - viewport));
        if (landed === top) return;
        top = landed;
        el.dispatchEvent(new Event("scroll"));
      },
    });
    return {
      to: (offset: number) => {
        el.scrollTop = offset;
      },
      at: () => top,
    };
  }

  it("stays where it was put when a message arrives, even opened from the activity board", () => {
    // The report: reading back through a cascade and being thrown to the end of
    // it, at times nobody could name. This was one of them. The transcript is
    // unmounted while the activity board is up, so the channel opened after it
    // is a different node, and the listener that noticed the operator scrolling
    // was still bound to the one that had been thrown away. Nothing reported a
    // scroll, so nothing had moved, so every message won.
    const rows = Array.from({ length: 12 }, () => envelope({}));
    useStore.setState({
      agents: [card(MANAGER, "Manager"), card(CHEF, "Chef")],
      messages: { [MANAGER]: rows },
      streams: {},
      reasoning: {},
      activity: {},
      lastActive: {},
      banner: null,
    });

    const { container, rerender } = render(
      <ChannelView
        channel={ACTIVITY_CHANNEL}
        onOpenMenu={() => {}}
        onNewAgent={() => {}}
        onNewGroup={() => {}}
      />,
    );
    rerender(
      <ChannelView
        channel={MANAGER}
        onOpenMenu={() => {}}
        onNewAgent={() => {}}
        onNewGroup={() => {}}
      />,
    );

    const scroller = container.querySelector<HTMLElement>(".pane__scroll");
    if (!scroller) throw new Error("no transcript to scroll");
    const box = measure(scroller, 4000, 400);
    // At the end, and then reading back through it.
    box.to(3600);
    box.to(1200);

    act(() => {
      useStore.setState({ messages: { [MANAGER]: [...rows, fromPeer("and another thing")] } });
    });

    expect(box.at()).toBe(1200);
  });

  it("stays where it was put after a pair thread has been read and closed", async () => {
    // The transcript is unmounted while a thread is up, so what comes back is a
    // different node. Whatever is watching the operator has to come back with
    // it, or this channel spends the rest of the session dragging them down.
    const rows = [
      envelope({ parts: [{ type: "text", text: "the operator's own message" }] }),
      fromPeer("hi"),
    ];
    const { container } = open(rows);

    fireEvent.click(screen.getByRole("button", { name: /Message from Chef/ }));
    await screen.findByText(/view-only/);
    fireEvent.click(screen.getByRole("button", { name: "Close chat" }));
    await waitFor(() => expect(screen.getByText("the operator's own message")).toBeTruthy());

    const scroller = container.querySelector<HTMLElement>(".pane__scroll");
    if (!scroller) throw new Error("no transcript to scroll");
    const box = measure(scroller, 4000, 400);
    box.to(3600);
    box.to(900);

    act(() => {
      useStore.setState({ messages: { [MANAGER]: [...rows, fromPeer("and another thing")] } });
    });

    expect(box.at()).toBe(900);
  });

  it("goes to the end when the operator sends a message from where they were", async () => {
    // The same rule read the other way. Typing into the box is a decision to be
    // at the end of the transcript: their own message landing off screen, with
    // nothing following it, is the same complaint pointing the other way.
    const { container } = open(Array.from({ length: 12 }, () => envelope({})));
    const scroller = container.querySelector<HTMLElement>(".pane__scroll");
    if (!scroller) throw new Error("no transcript to scroll");
    const box = measure(scroller, 4000, 400);
    box.to(3600);
    box.to(1200);

    await act(async () => {
      fireEvent.change(screen.getByPlaceholderText("Message Manager"), {
        target: { value: "carry on" },
      });
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Send" }));
    });

    expect(box.at()).toBe(3600);
  });
});
