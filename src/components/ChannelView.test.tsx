import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useEffect, useState } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DEFAULT_PREFS } from "../lib/prefs";
import { useStore } from "../lib/store";
import type { LiveCall } from "../lib/trail";
import type { Activity, AgentCard, Envelope, MessageId, Part, ToolOutcome } from "../lib/types";
import { ChannelView } from "./ChannelView";
import { Search } from "./Search";

const pairMessages = vi.fn<() => Promise<Envelope[]>>(async () => []);
const sendMessage = vi.fn(async (..._args: unknown[]) => ({ messageId: "message", runId: "run" }));

vi.mock("../lib/ipc", () => ({
  api: {
    channelMessages: async () => [],
    conversationFlow: async () => [],
    clearChannel: async () => 0,
    sendMessage: (...args: unknown[]) => sendMessage(...args),
    agentComputer: async () => null,
    pairMessages: () => pairMessages(),
    search: async () => ({ messages: [], files: [], links: [], routines: [], artifacts: [] }),
    claudeProgram: async () => ({ installed: false }),
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
    runsErrands: false,
    browserConsent: "open",
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
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

function Workspace() {
  const [overlay, setOverlay] = useState<"search" | null>(null);
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.ctrlKey) return;
      if (event.key === "k") setOverlay("search");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return (
    <>
      <ChannelView channel={MANAGER} onOpenMenu={() => {}} />
      {overlay === "search" && (
        <Search
          onClose={() => setOverlay(null)}
          onEditAgent={() => {}}
          onEditGroup={() => {}}
          onNewAgent={() => {}}
          onNewGroup={() => {}}
          onOpenCafeteria={() => {}}
          onOpenSettings={() => {}}
        />
      )}
    </>
  );
}

function open(messages: Envelope[], withOverlays = false) {
  useStore.setState({
    prefs: DEFAULT_PREFS,
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
    withOverlays ? <Workspace /> : <ChannelView channel={MANAGER} onOpenMenu={() => {}} />,
  );
}

beforeEach(() => {
  sessionStorage.clear();
  sendMessage.mockReset();
  sendMessage.mockResolvedValue({ messageId: "message", runId: "run" });
  pairMessages.mockClear();
  pairMessages.mockResolvedValue([]);
  useStore.setState({ focused: null });
});

it("passes the composer key to the host and returns its receipt", async () => {
  open([]);
  const input = screen.getByPlaceholderText("Write to Manager…");
  fireEvent.change(input, { target: { value: "first" } });
  fireEvent.click(screen.getByRole("button", { name: "Send" }));
  await waitFor(() =>
    expect(sendMessage).toHaveBeenCalledWith(MANAGER, "first", [], expect.any(String)),
  );
  sendMessage.mockRejectedValueOnce(new Error("lost"));
  await waitFor(() =>
    expect((screen.getByRole("button", { name: "Send" }) as HTMLButtonElement).disabled).toBe(true),
  );
  fireEvent.change(input, { target: { value: "second" } });
  fireEvent.click(screen.getByRole("button", { name: "Send" }));
  expect(await screen.findByRole("button", { name: "Send again" })).toBeTruthy();
});

describe("peer traffic in a channel", () => {
  it("returns to the exact peer opener when that peer has multiple bursts", async () => {
    open([fromPeer("first"), envelope({}), fromPeer("second")]);
    const openers = screen.getAllByRole("button", { name: /Message from Chef/ });
    expect(openers).toHaveLength(2);
    fireEvent.click(openers[1]!);
    await screen.findByText(/view-only/);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(document.activeElement).toBe(
      screen.getAllByRole("button", { name: /Message from Chef/ })[1],
    );
  });

  it("gives each peer in a burst a distinct stable opener", async () => {
    const other = "00000000-0000-4000-8000-0000000000a3";
    open([
      fromPeer("first"),
      envelope({
        from: { kind: "agent", id: other },
        to: { kind: "agent", id: MANAGER },
        parts: [{ type: "text", text: "second" }],
      }),
    ]);
    act(() =>
      useStore.setState({
        agents: [card(MANAGER, "Manager"), card(CHEF, "Chef"), card(other, "Baker")],
      }),
    );
    const chef = screen.getByRole("button", { name: /Message from Chef/ });
    const baker = screen.getByRole("button", { name: /Message from Baker/ });
    const chefId = chef.dataset.dismissId;
    const bakerId = baker.dataset.dismissId;
    expect(chefId).toBeTruthy();
    expect(bakerId).toBeTruthy();
    expect(chefId).not.toBe(bakerId);
    fireEvent.click(baker);
    await screen.findByText(/view-only/);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.getByRole("button", { name: /Message from Chef/ }).dataset.dismissId).toBe(
      chefId,
    );
    expect(screen.getByRole("button", { name: /Message from Baker/ }).dataset.dismissId).toBe(
      bakerId,
    );
    expect(document.activeElement).toBe(screen.getByRole("button", { name: /Message from Baker/ }));
  });

  it.each([["Search", "k"]])(
    "dismisses %s above the pair thread before closing the thread",
    async (name, key) => {
      open([fromPeer("hi")], true);
      const opener = screen.getByRole("button", { name: /Message from Chef/ });
      opener.focus();
      fireEvent.click(opener);
      await screen.findByText(/view-only/);
      const close = screen.getByRole("button", { name: "Close chat" });
      close.focus();
      fireEvent.keyDown(window, { key, ctrlKey: true });
      const dialog = await screen.findByRole("dialog", { name });
      expect(dialog.contains(document.activeElement)).toBe(true);
      fireEvent.keyDown(window, { key: "Escape" });
      expect(dialog.isConnected).toBe(false);
      expect(screen.getByRole("button", { name: "Close chat" })).toBe(close);
      expect(document.activeElement).toBe(close);
      fireEvent.keyDown(window, { key: "Escape" });
      expect(screen.queryByRole("button", { name: "Close chat" })).toBeNull();
      expect(document.activeElement).toBe(
        screen.getByRole("button", { name: /Message from Chef/ }),
      );
    },
  );
  it("shows one line for a burst, not one per message", () => {
    open([
      envelope({}),
      fromPeer("a long report nobody asked to read here"),
      fromPeer("and another"),
    ]);

    expect(screen.getByRole("button", { name: /2 messages with Chef/ })).toBeTruthy();
    expect(screen.queryByText("and another")).toBeNull();
    expect(document.querySelectorAll(".footnote")).toHaveLength(1);
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

    expect(await screen.findByText("what is the status")).toBeTruthy();
    expect(screen.getByText("all clear")).toBeTruthy();
    expect(screen.getByText(/view-only/)).toBeTruthy();
  });

  it("does not offer to send anything from inside that thread", async () => {
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
  it("names the speaker once per turn rather than once per message", () => {
    useStore.setState({
      running: [{ runId: "r1", agentId: MANAGER, groupId: "g", startedAt: 0, note: null }],
      runningKnown: true,
    });
    open([
      envelope({ parts: [{ type: "text", text: "what is the status" }] }),
      envelope({
        from: { kind: "agent", id: MANAGER },
        to: { kind: "human" },
        parts: [{ type: "text", text: "all clear" }],
      }),
      envelope({
        from: { kind: "agent", id: MANAGER },
        to: { kind: "human" },
        parts: [{ type: "text", text: "and done" }],
      }),
    ]);

    expect(screen.getByText("all clear")).toBeTruthy();
    expect(screen.getAllByText("Manager", { selector: "h1, .msg__name" })).toHaveLength(2);
    expect(document.querySelectorAll(".msg--continued")).toHaveLength(1);
    expect(document.querySelectorAll(".msg__head")).toHaveLength(1);
    expect(document.querySelectorAll(".msg__bubble")).toHaveLength(1);
    useStore.setState({ running: [] });
  });

  it("sets the line a settled conversation ended on apart as its result, with its own byline", () => {
    useStore.setState({ running: [], runningKnown: true });
    open([
      envelope({ parts: [{ type: "text", text: "what is the status" }] }),
      envelope({
        from: { kind: "agent", id: MANAGER },
        to: { kind: "human" },
        parts: [{ type: "text", text: "checking with Chef" }],
      }),
      envelope({
        from: { kind: "agent", id: MANAGER },
        to: { kind: "human" },
        parts: [{ type: "text", text: "all clear" }],
      }),
    ]);
    const result = screen.getByRole("article", { name: /^Result from Manager/ });
    expect(result.textContent).toContain("all clear");
    expect(result.querySelector(".msg__name")?.textContent).toBe("Manager");
    expect(screen.getByText("checking with Chef").closest(".msg--result")).toBeNull();
  });

  it("draws no result while the conversation is still running", () => {
    useStore.setState({
      running: [{ runId: "r1", agentId: MANAGER, groupId: "g", startedAt: 0, note: null }],
      runningKnown: true,
    });
    open([
      envelope({ parts: [{ type: "text", text: "what is the status" }] }),
      envelope({
        from: { kind: "agent", id: MANAGER },
        to: { kind: "human" },
        parts: [{ type: "text", text: "checking with Chef" }],
      }),
    ]);
    expect(screen.queryByRole("article", { name: /^Result from/ })).toBeNull();
    useStore.setState({ running: [] });
  });

  it("draws no result before the host has said what is running", () => {
    useStore.setState({ running: [], runningKnown: false });
    open([
      envelope({ parts: [{ type: "text", text: "what is the status" }] }),
      envelope({
        from: { kind: "agent", id: MANAGER },
        to: { kind: "human" },
        parts: [{ type: "text", text: "checking with Chef" }],
      }),
    ]);
    expect(screen.queryByRole("article", { name: /^Result from/ })).toBeNull();
    act(() => useStore.setState({ runningKnown: true }));
    expect(screen.getByRole("article", { name: /^Result from Manager/ })).toBeTruthy();
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
    expect(container.querySelectorAll(".when-row")).toHaveLength(1);
  });
});

describe("what the head says answers", () => {
  it("names the model once, by the name a person reads, and never by its id", () => {
    const { container } = open([envelope({ parts: [{ type: "text", text: "hi" }] })]);
    act(() =>
      useStore.setState((state) => ({
        agents: state.agents.map((agent) =>
          agent.id === MANAGER ? { ...agent, model: "anthropic/claude-sonnet-4.5" } : agent,
        ),
      })),
    );
    expect(container.querySelector(".pane__meta")?.textContent).toBe("Claude Sonnet 4.5");
    expect(container.textContent).not.toContain("anthropic/claude-sonnet-4.5");
    expect(container.textContent?.split("Claude Sonnet 4.5")).toHaveLength(2);
  });
});

describe("while an agent is working", () => {
  const doing = (state: Activity) =>
    act(() => useStore.setState({ activity: { [MANAGER]: state } }));

  it("says so above the composer, naming it", () => {
    open([envelope({})]);
    expect(screen.queryByText("Manager is working")).toBeNull();

    doing({ state: "thinking" });
    expect(screen.getByText("Manager is working")).toBeTruthy();

    doing({ state: "idle" });
    expect(screen.queryByText("Manager is working")).toBeNull();
  });

  it("shows what it is thinking, and stops when the thought is dropped", () => {
    open([envelope({})]);
    doing({ state: "thinking" });

    act(() => useStore.setState({ reasoning: { [MANAGER]: "**Checking**\n\nthe totals agree." } }));
    expect(screen.getByText("Checking")).toBeTruthy();
    expect(screen.getByText("the totals agree.")).toBeTruthy();
    expect(screen.queryByText("Manager is working")).toBeNull();
    expect(document.querySelector(".working")?.getAttribute("role")).toBeNull();

    act(() => useStore.setState({ reasoning: {} }));
    const note = document.querySelector(".working");
    expect(note?.getAttribute("role")).toBe("status");
    expect(note?.textContent).toContain("Manager is working");
  });

  it("draws the heading alone until a sentence under it has finished", () => {
    open([envelope({})]);
    doing({ state: "thinking" });
    act(() => useStore.setState({ reasoning: { [MANAGER]: "**Checking**\n\nthe totals do" } }));

    expect(screen.getByText("Checking")).toBeTruthy();
    expect(screen.queryByText("the totals do")).toBeNull();
  });

  it("opens the whole of the working, and closes it when the turn ends", () => {
    open([envelope({})]);
    doing({ state: "thinking" });
    const held = "**Checking**\n\nthe totals agree.\n\nSo the third quarter is the one to redo.";
    act(() => useStore.setState({ reasoning: { [MANAGER]: held } }));

    expect(document.querySelector(".thought")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Show what Manager is working through/ }));
    expect(document.querySelector(".thought__text")?.textContent).toBe(held);

    doing({ state: "idle" });
    doing({ state: "thinking" });
    expect(document.querySelector(".thought")).toBeNull();
  });

  it("offers nothing to open where the model published nothing", () => {
    open([envelope({})]);
    doing({ state: "thinking" });

    expect(screen.queryByRole("button", { name: /working through/ })).toBeNull();
  });

  const finished = (
    callId: string,
    name: string,
    args: object,
    outcome: ToolOutcome,
  ): LiveCall => ({
    callId,
    name,
    arguments: args,
    done: { type: "toolCall", name, arguments: args, outcome },
    startedAt: 0,
  });

  const browsed = (callId: string) =>
    finished(
      callId,
      "browse",
      { action: "open", url: "https://www.cnn.com/world" },
      {
        status: "ok",
        summary: "read cnn.com",
      },
    );

  it("counts the calls the turn has made, and opens them as the transcript will", () => {
    open([envelope({})]);
    doing({ state: "thinking" });
    act(() => useStore.setState({ trail: { [MANAGER]: [browsed("call_1")] } }));

    expect(screen.queryByText("Opened cnn.com")).toBeNull();
    const count = screen.getByRole("button", { name: /1 step/ });
    fireEvent.click(count);
    expect(screen.getByText("Opened cnn.com")).toBeTruthy();

    act(() => useStore.setState({ trail: { [MANAGER]: [browsed("call_1"), browsed("call_2")] } }));
    expect(document.querySelector(".working__steps")?.textContent).toContain("2 steps");
  });

  it("marks the count where something went wrong, and says how much", () => {
    open([envelope({})]);
    doing({ state: "thinking" });
    act(() =>
      useStore.setState({
        trail: {
          [MANAGER]: [
            browsed("call_1"),
            finished(
              "call_2",
              "run_command",
              { command: "boom" },
              {
                status: "failed",
                error: "no machine",
              },
            ),
          ],
        },
      }),
    );

    const count = screen.getByRole("button", { name: /2 steps, 1 failed/ });
    expect(count.getAttribute("data-failed")).toBe("true");
  });

  it("keeps a spent credential on the line, never behind the click", () => {
    open([envelope({})]);
    doing({ state: "thinking" });
    act(() =>
      useStore.setState({
        trail: {
          [MANAGER]: [
            finished(
              "call_1",
              "run_command",
              { command: "curl" },
              {
                status: "ok",
                summary: "used Mistral ($MISTRAL_API_KEY) · exit 0",
              },
            ),
          ],
        },
      }),
    );

    expect(document.querySelector(".working__end .trail__spent")?.textContent).toBe(
      "Mistral ($MISTRAL_API_KEY)",
    );
  });

  it("gives the working and the calls one slot, so one closes the other", () => {
    open([envelope({})]);
    doing({ state: "thinking" });
    act(() =>
      useStore.setState({
        reasoning: { [MANAGER]: "**Checking**\n\nthe totals agree." },
        trail: { [MANAGER]: [browsed("call_1")] },
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: /1 step/ }));
    expect(document.querySelector(".steps")).toBeTruthy();
    expect(document.querySelector(".thought")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /Show what Manager is working through/ }));
    expect(document.querySelector(".thought")).toBeTruthy();
    expect(document.querySelector(".steps")).toBeNull();
  });

  it("says what it is waiting on, and how long it has been", () => {
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
              startedAt: Date.now() - 74_500,
            },
          ],
        },
      }),
    );

    expect(screen.getByText("Running a command")).toBeTruthy();
    expect(screen.getByText("1m 14s")).toBeTruthy();
    expect(screen.queryByText("I should check the tests.")).toBeNull();
    expect(screen.getByRole("button", { name: /working through/ })).toBeTruthy();
  });

  it("leaves the line alone for a call that answers immediately", () => {
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
    expect(container.querySelectorAll("[data-found='true']")).toHaveLength(1);
    scrolled.mockRestore();
  });

  it("stops being marked once it has been shown", async () => {
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
    const missing = "00000000-0000-4000-8000-999999999999" as MessageId;
    useStore.setState({ focused: missing });
    const { container } = open(Array.from({ length: 12 }, () => envelope({})));

    await waitFor(() => expect(container.querySelectorAll("[data-message]")).toHaveLength(12));
    expect(container.querySelector("[data-found='true']")).toBeNull();
    expect(useStore.getState().focused).toBe(missing);
  });

  it("lands on the row that swallowed a message between two agents", async () => {
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

  async function settle(next: () => Envelope[]) {
    await act(async () => {
      useStore.setState({ messages: { [MANAGER]: next() } });
      await new Promise((resolve) => requestAnimationFrame(() => resolve(null)));
    });
  }

  it("stays where it was put after a pair thread has been read and closed", async () => {
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

    await settle(() => [...rows, fromPeer("something newer")]);
    expect(box.at()).toBe(3600);

    box.to(900);

    await settle(() => [...rows, fromPeer("something newer"), fromPeer("and another thing")]);

    expect(box.at()).toBe(900);
  });

  it("goes to the end when the operator sends a message from where they were", async () => {
    const { container } = open(Array.from({ length: 12 }, () => envelope({})));
    const scroller = container.querySelector<HTMLElement>(".pane__scroll");
    if (!scroller) throw new Error("no transcript to scroll");
    const box = measure(scroller, 4000, 400);
    box.to(3600);
    box.to(1200);

    await act(async () => {
      fireEvent.change(screen.getByPlaceholderText("Write to Manager…"), {
        target: { value: "carry on" },
      });
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Send" }));
    });

    expect(box.at()).toBe(3600);
  });
});

describe("inline reasoning and tool calls", () => {
  const messageId = "inline-stream";
  function start(agentId = MANAGER) {
    useStore.getState().applyEvent({
      type: "streamStarted",
      messageId,
      agentId,
      channelId: MANAGER,
      runId: "inline-run",
      to: { kind: "human" },
    });
  }
  function think(text: string) {
    useStore.getState().applyEvent({ type: "reasoningDelta", messageId, text });
  }
  function call() {
    useStore.getState().applyEvent({
      type: "toolStarted",
      messageId,
      callId: "command",
      name: "run_command",
      arguments: { command: "pnpm test" },
    });
  }

  it("puts full thinking and each running call in chat in arrival order", () => {
    open([envelope({})]);
    act(() => {
      useStore.getState().setPrefs({ showReasoning: true });
      start();
      think("Check the existing tests.\nRead every failure.");
      call();
      think("Now inspect the result.");
    });
    const log = screen.getByRole("log");
    const work = screen.getByRole("region", { name: "Reasoning and tool calls" });
    expect(log.contains(work)).toBe(true);
    const entries = work.querySelectorAll("details");
    expect(entries[0]?.textContent).toContain("Read every failure.");
    expect(entries[1]?.textContent).toContain("run_command");
    expect(entries[1]?.textContent).toContain("Running");
    expect(entries[2]?.textContent).toContain("Now inspect the result.");
    expect(entries[1]?.querySelector("pre")?.textContent).toContain("pnpm test");
    expect(work.getAttribute("aria-live")).toBe("off");
    expect(document.querySelector(".thought")).toBeNull();
  });

  it.each([
    [{ status: "failed", error: "Command timed out" }, "Failed", "Command timed out"],
    [{ status: "refused", reason: "Permission denied" }, "Refused", "Permission denied"],
    [
      { status: "partial", summary: "One delivered", refused: [] },
      "Partially completed",
      "One delivered",
    ],
    [{ status: "ok", summary: "All tests passed" }, "Done", "All tests passed"],
  ] as const)(
    "shows the actual outcome %j without requiring reasoning",
    (outcome, label, detail) => {
      open([]);
      act(() => {
        useStore.getState().setPrefs({ showReasoning: true });
        start();
        call();
        useStore.getState().applyEvent({
          type: "toolFinished",
          messageId,
          callId: "command",
          part: {
            type: "toolCall",
            name: "run_command",
            arguments: { command: "pnpm test" },
            outcome: outcome as ToolOutcome,
          },
        });
      });
      expect(document.querySelector(".turn-work__status")?.textContent).toBe(label);
      expect(document.querySelector(".turn-work__description")?.textContent).toContain(detail);
      expect(document.querySelector(".turn-work__thinking")).toBeNull();
    },
  );

  it("honors toggling mid-turn and clears work on retry and completion", () => {
    open([]);
    act(() => {
      start();
      think("First attempt.");
      call();
    });
    expect(document.querySelector(".turn-work")).toBeNull();
    act(() => useStore.getState().setPrefs({ showReasoning: true }));
    expect(document.querySelector(".turn-work")?.textContent).toContain("First attempt.");
    act(() => useStore.getState().setPrefs({ showReasoning: false }));
    expect(document.querySelector(".turn-work")).toBeNull();
    act(() => {
      useStore.getState().setPrefs({ showReasoning: true });
      useStore.getState().applyEvent({ type: "streamEnded", messageId, channelId: MANAGER });
      start();
      think("Retry working.");
    });
    expect(document.querySelector(".turn-work")?.textContent).not.toContain("First attempt.");
    expect(document.querySelector(".turn-work")?.textContent).toContain("Retry working.");
    act(() =>
      useStore.getState().applyEvent({ type: "streamEnded", messageId, channelId: MANAGER }),
    );
    expect(document.querySelector(".turn-work")).toBeNull();
  });

  it("shows the channel owner's work, even when another agent writes here", () => {
    open([]);
    act(() => {
      useStore.getState().setPrefs({ showReasoning: true });
      start(CHEF);
      think("Chef's working.");
      call();
    });
    expect(document.querySelector(".turn-work")).toBeNull();
  });
});

describe("what a screen reader is told", () => {
  const said = () => document.querySelector('p.offscreen[role="status"]')?.textContent;
  const toOperator = (from: string, channelId: string, text: string) =>
    envelope({
      channelId,
      from: { kind: "agent", id: from },
      to: { kind: "human" },
      parts: [{ type: "text", text }],
    });

  it("says nothing on a channel switch, and the next arrival there", () => {
    const { rerender } = open([toOperator(MANAGER, MANAGER, "Filed it.")]);
    const chef = [toOperator(CHEF, CHEF, "Menu is set.")];
    act(() => useStore.setState({ messages: { ...useStore.getState().messages, [CHEF]: chef } }));
    rerender(<ChannelView channel={CHEF} onOpenMenu={() => {}} />);
    expect(said()).toBe("");

    act(() =>
      useStore.setState({
        messages: {
          ...useStore.getState().messages,
          [CHEF]: [...chef, toOperator(CHEF, CHEF, "Dinner is at seven.")],
        },
      }),
    );
    expect(said()).toBe("Chef: Dinner is at seven.");
  });
});
