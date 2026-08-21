import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { Lookups } from "../lib/transcript";
import type { AgentCard, Envelope, Part } from "../lib/types";
import { MessageItem } from "./MessageItem";

const retryTurn = vi.fn<(agentId: string, messageId: string) => Promise<string>>(
  async () => "run-2",
);
vi.mock("../lib/ipc", () => ({
  openExternal: vi.fn(),
  api: { retryTurn: (agentId: string, messageId: string) => retryTurn(agentId, messageId) },
}));

const openRoutine = vi.fn<(id: string) => void>();
vi.mock("../lib/store", () => ({
  useStore: { getState: () => ({ openRoutine, setBanner: vi.fn() }) },
}));

function card(id: string, name: string): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
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

const AGENTS = [card("manager", "Manager"), card("chef", "Chef")];

const lookups: Lookups = {
  byId: (id) => AGENTS.find((a) => a.id === id),
  byName: (name) => AGENTS.find((a) => a.name.toLowerCase() === name.toLowerCase()),
};

function envelope(overrides: Partial<Envelope>): Envelope {
  return {
    id: "m1",
    runId: "r1",
    channelId: "manager",
    from: { kind: "human" },
    to: { kind: "agent", id: "manager" },
    parts: [{ type: "text", text: "hello there" }],
    trust: "operator",
    hop: 0,
    expectsReply: true,
    intent: "courtesy",
    cause: null,
    createdAt: 1_700_000_000_000,
    ...overrides,
  };
}

function show(message: Envelope) {
  return render(<MessageItem message={message} lookups={lookups} continued={false} />);
}

describe("messages addressed to the operator", () => {
  it("renders what you said as a bubble, with no avatar", () => {
    const { container } = show(envelope({}));
    expect(screen.getByText("hello there")).toBeTruthy();
    // You know who you are; the avatars exist to tell the agents apart.
    expect(container.querySelector(".avatar")).toBeNull();
  });

  it("renders an agent's reply to you as a bubble, with an avatar", () => {
    const { container } = show(
      envelope({
        from: { kind: "agent", id: "manager" },
        to: { kind: "human" },
        parts: [{ type: "text", text: "on it" }],
      }),
    );
    expect(screen.getByText("on it")).toBeTruthy();
    expect(container.querySelector(".avatar")).toBeTruthy();
  });
});

describe("naming the author", () => {
  const reply = envelope({
    from: { kind: "agent", id: "manager" },
    to: { kind: "human" },
    parts: [{ type: "text", text: "on it" }],
  });

  it("writes it out where two agents look alike in a list", () => {
    // The pair's own thread. Neither participant is the person reading, and
    // one column of text with two authors in it is unreadable without names.
    render(<MessageItem message={reply} lookups={lookups} continued={false} named />);
    expect(screen.getByText("Manager")).toBeTruthy();
  });

  it("leaves it out where the channel has already answered the question", () => {
    // An agent's own channel has two participants: the one named at the top of
    // the pane, and the operator. A name over every message is the loudest
    // thing on the page and the one that says least.
    const { container } = render(
      <MessageItem message={reply} lookups={lookups} continued={false} named={false} />,
    );
    expect(screen.queryByText("Manager")).toBeNull();
    // The portrait still says which agent, and it is one glyph rather than a
    // line of display type over every paragraph.
    expect(container.querySelector(".avatar")).toBeTruthy();
  });

  it("keeps your own message unmistakable without naming you either", () => {
    const { container } = render(
      <MessageItem message={envelope({})} lookups={lookups} continued={false} named={false} />,
    );
    expect(screen.queryByText("You")).toBeNull();
    expect(container.querySelector(".msg[data-operator='true']")).toBeTruthy();
    expect(container.querySelector(".avatar")).toBeNull();
  });

  it("still carries the time, out of the way", () => {
    // Off the header and onto the row itself: every message has one and almost
    // none of them are worth a line. It is a hover, and the transcript draws a
    // line of its own wherever the gap was long enough to matter.
    const { container } = render(
      <MessageItem message={reply} lookups={lookups} continued={false} named={false} />,
    );
    const at = container.querySelector("time.msg__at");
    expect(at).toBeTruthy();
    expect(at?.getAttribute("datetime")).toBe(new Date(reply.createdAt).toISOString());
  });
});

describe("agent-to-agent traffic", () => {
  it("is a bubble here, because the only place it is drawn one by one is the pair's own thread", () => {
    // In a channel these never reach this component: the transcript collapses
    // them into a burst row first. What is left is the thread the operator
    // opened off that row, which they opened in order to read.
    show(
      envelope({
        channelId: "chef",
        from: { kind: "agent", id: "manager" },
        to: { kind: "agent", id: "chef" },
        hop: 2,
        parts: [{ type: "text", text: "a very long briefing document" }],
      }),
    );
    expect(screen.getByText("a very long briefing document")).toBeTruthy();
    expect(screen.getByText("Manager")).toBeTruthy();
  });
});

describe("an agent's own record of what it did", () => {
  const record = (part: Part) =>
    envelope({
      from: { kind: "agent", id: "manager" },
      to: { kind: "system" },
      trust: "system",
      parts: [part],
    });

  it("keeps a directory lookup quiet and unclickable", () => {
    show(
      record({
        type: "toolCall",
        name: "directory",
        arguments: {},
        outcome: { status: "ok", summary: "2 agent(s): Chef, Scribe" },
      }),
    );
    // Sentence-cased and unnamed: there is exactly one agent whose own work
    // this can be, and its name is at the top of the pane. The row this
    // replaced put that name in front of every line of it.
    expect(screen.getByText("Checked who is available")).toBeTruthy();
    // Nothing behind it, so nothing to press. A control that opens nothing is
    // one the operator stops trusting the rest of.
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("does not draw a memory update as a message to nobody", () => {
    // update_notes has no recipients, so falling through to the send renderer
    // drew it as "Sent to no one" with the memory body as the message.
    show(
      record({
        type: "toolCall",
        name: "update_notes",
        arguments: { content: "Smith handles verification." },
        outcome: { status: "ok", summary: "Memory saved (28 characters)." },
      }),
    );
    expect(screen.queryByText(/no one/)).toBeNull();
    expect(screen.getByText("Updated its memory")).toBeTruthy();

    // And what it wrote is one click away, which is the whole of what an
    // operator wants from this row and the one thing it never showed.
    fireEvent.click(screen.getByRole("button", { name: /Updated its memory/ }));
    expect(screen.getByText("Smith handles verification.")).toBeTruthy();
  });

  it("names an unrecognised tool rather than guessing it was a send", () => {
    show(
      record({
        type: "toolCall",
        name: "run_code",
        arguments: { source: "print(1)" },
        outcome: { status: "ok", summary: "exit 0" },
      }),
    );
    expect(screen.queryByText(/no one/)).toBeNull();
    expect(screen.getByText("Used run_code")).toBeTruthy();
  });

  it("says why a tool call failed, not just that it happened", () => {
    // The one send that lands here is the one naming nobody, and it is exactly
    // the one where the reason is the whole of what there is to see. A line
    // saying only "Manager used send_message" describes a working app.
    show(
      record({
        type: "toolCall",
        name: "send_message",
        arguments: { text: "hello?" },
        outcome: { status: "refused", reason: "Refused: name a recipient." },
      }),
    );
    expect(screen.getByText(/name a recipient/)).toBeTruthy();
  });

  it("surfaces a guard stop as a centred notice", () => {
    show(record({ type: "notice", kind: "guardStop", text: "hop limit (8) reached" }));
    expect(screen.getByText("hop limit (8) reached")).toBeTruthy();
  });
});

describe("a failed turn", () => {
  /** What the runtime writes once its own retries are spent. */
  function failure(cause: string | null): Envelope {
    return envelope({
      id: "notice-1",
      from: { kind: "system" },
      to: { kind: "agent", id: "manager" },
      trust: "system",
      expectsReply: false,
      cause,
      parts: [
        {
          type: "notice",
          kind: "upstreamError",
          text: "Manager could not reply: could not reach the inference endpoint",
        },
      ],
    });
  }

  it("offers to send the message again, and says which", () => {
    retryTurn.mockClear();
    render(<MessageItem message={failure("m-original")} lookups={lookups} continued={false} />);

    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(retryTurn).toHaveBeenCalledWith("manager", "m-original");
    // And it will not fire twice on a double click: a second run is a second
    // model call, billed.
    expect((screen.getByRole("button", { name: "Sent again" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
  });

  it("offers nothing when there is nothing to send again", () => {
    render(<MessageItem message={failure(null)} lookups={lookups} continued={false} />);
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
  });

  it("does not offer a retry for a limit that would be hit again", () => {
    // The guard refused this on purpose. A button here would spend the same
    // budget to reach the same refusal.
    const stopped = envelope({
      from: { kind: "system" },
      to: { kind: "agent", id: "manager" },
      cause: "m-original",
      parts: [{ type: "notice", kind: "guardStop", text: "this conversation used its budget" }],
    });
    render(<MessageItem message={stopped} lookups={lookups} continued={false} />);
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
  });
});

describe("a command that used a credential", () => {
  it("says so in the transcript, by name, with no value anywhere", () => {
    // The operator's audit trail for their own tokens. Before this, a
    // credential went into the environment of every command and nothing
    // distinguished the command that spent it.
    const used = envelope({
      from: { kind: "agent", id: "manager" },
      to: { kind: "system" },
      parts: [
        {
          type: "toolCall",
          name: "run_command",
          arguments: { command: 'curl -H "Authorization: Bearer $MISTRAL_API_KEY" ...' },
          outcome: {
            status: "ok",
            summary: "used Mistral ($MISTRAL_API_KEY) · exit 0, 812 bytes out",
          },
        },
      ],
    });
    const { container } = render(
      <MessageItem message={used} lookups={lookups} continued={false} />,
    );

    // On the row itself, never behind a click and never folded into a count:
    // this is the operator's audit trail for their own tokens.
    expect(screen.getByText("Mistral ($MISTRAL_API_KEY)")).toBeTruthy();
    // The exit code and the command are behind the click, which is where an
    // exit code is worth reading. The credential is not, which is the whole
    // point of it.
    expect(container.textContent).not.toContain("exit 0, 812 bytes out");
    expect(container.textContent).not.toContain("curl -H");

    fireEvent.click(screen.getByRole("button", { name: /Ran a command/ }));
    expect(container.textContent).toContain("exit 0, 812 bytes out");
    expect(screen.getByText(/Authorization: Bearer \$MISTRAL_API_KEY/)).toBeTruthy();
  });
});

describe("a turn that used its computer two dozen times", () => {
  /** What the runtime writes for one turn: every call in one envelope. */
  function browsing(): Envelope {
    return envelope({
      from: { kind: "agent", id: "manager" },
      to: { kind: "system" },
      trust: "system",
      parts: [
        {
          type: "toolCall",
          name: "browse",
          arguments: { action: "open", url: "https://cnn.com" },
          outcome: { status: "ok", summary: "open in the browser" },
        },
        {
          type: "toolCall",
          name: "browse",
          arguments: { action: "read" },
          outcome: { status: "ok", summary: "read in the browser" },
        },
        {
          type: "toolCall",
          name: "browse",
          arguments: { action: "click", id: 12 },
          outcome: { status: "ok", summary: "click in the browser" },
        },
      ],
    });
  }

  it("draws one line for the run of them, naming where it went", () => {
    // Twenty-four rounds is the limit and a browsing turn spends most of it.
    // A line apiece is the same burial peer traffic was collapsed to fix.
    const { container } = render(
      <MessageItem message={browsing()} lookups={lookups} continued={false} />,
    );
    expect(screen.getByRole("button", { name: /3 steps on cnn.com/ })).toBeTruthy();
    expect(container.querySelectorAll(".trail__chip")).toHaveLength(1);
  });

  it("opens the calls behind it, in order, and closes again", () => {
    render(<MessageItem message={browsing()} lookups={lookups} continued={false} />);
    const chip = screen.getByRole("button", { name: /3 steps on cnn.com/ });
    expect(chip.getAttribute("aria-expanded")).toBe("false");

    fireEvent.click(chip);
    expect(chip.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("Opened cnn.com")).toBeTruthy();
    expect(screen.getByText("Read the page")).toBeTruthy();
    expect(screen.getByText("Clicked on the page")).toBeTruthy();

    fireEvent.click(chip);
    expect(screen.queryByText("Read the page")).toBeNull();
  });
});

describe("redrawing a transcript", () => {
  it("does not draw an entry again when nothing about it changed", () => {
    // A transcript is rebuilt whenever any message is appended, and drawing an
    // entry parses its markdown. Ten agents reporting at once meant every
    // message on screen re-parsed for each arrival, which is the other half of
    // what made the window stop responding.
    const message = envelope({ parts: [{ type: "text", text: "the answer is 42" }] });
    const view = render(<MessageItem message={message} lookups={lookups} continued={false} />);
    const drawn = screen.getByText("the answer is 42");

    // The same envelope, the same lookups: a parent redraw with nothing new.
    view.rerender(<MessageItem message={message} lookups={lookups} continued={false} />);

    // The node survives rather than being replaced, which is what memoisation
    // buys: React never called the component at all.
    expect(screen.getByText("the answer is 42")).toBe(drawn);
  });

  it("still redraws when the message itself changes", () => {
    const view = render(
      <MessageItem
        message={envelope({ parts: [{ type: "text", text: "first" }] })}
        lookups={lookups}
        continued={false}
      />,
    );
    view.rerender(
      <MessageItem
        message={envelope({ parts: [{ type: "text", text: "second" }] })}
        lookups={lookups}
        continued={false}
      />,
    );

    expect(screen.getByText("second")).toBeTruthy();
    expect(screen.queryByText("first")).toBeNull();
  });
});

describe("a routine coming due", () => {
  const fired = (parts?: Part[]) =>
    envelope({
      from: { kind: "system" },
      to: { kind: "agent", id: "manager" },
      intent: "work",
      parts: parts ?? [
        {
          type: "routine",
          routineId: "rt1",
          name: "Listings sweep",
          what: "Check the listings. America/New_York. Post one copy and say what changed.",
        },
      ],
    });

  it("is one line naming the routine, not the instruction as dialogue", () => {
    // What this replaces: a chat bubble from "Guaca" carrying several
    // sentences of instruction, in the middle of the operator's own
    // conversation with the agent.
    show(fired());

    expect(screen.getByText("Listings sweep")).toBeTruthy();
    expect(screen.getByText("routine ran")).toBeTruthy();
    expect(screen.queryByText(/America\/New_York/)).toBeNull();
    expect(screen.queryByText("Guaca")).toBeNull();
  });

  it("titles an unnamed routine by what it says, like the schedule panel does", () => {
    show(
      fired([
        { type: "routine", routineId: "rt1", name: "", what: "Check the listings. Then post." },
      ]),
    );
    expect(screen.getByText("Check the listings")).toBeTruthy();
  });

  it("opens the routine it names", () => {
    // The panel that draws a routine is the transcript's sibling, so the click
    // asks for it through the store rather than through a prop on every row.
    show(fired());
    fireEvent.click(screen.getByRole("button"));
    expect(openRoutine).toHaveBeenCalledWith("rt1");
  });
});
