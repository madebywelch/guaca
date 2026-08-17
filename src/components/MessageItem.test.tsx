import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AgentCard, Envelope, Part } from "../lib/types";
import { type Lookups, MessageItem } from "./MessageItem";

const retryTurn = vi.fn<(agentId: string, messageId: string) => Promise<string>>(
  async () => "run-2",
);
vi.mock("../lib/ipc", () => ({
  openExternal: vi.fn(),
  api: { retryTurn: (agentId: string, messageId: string) => retryTurn(agentId, messageId) },
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

function show(message: Envelope, feed = false) {
  return render(<MessageItem message={message} lookups={lookups} continued={false} feed={feed} />);
}

describe("messages addressed to the operator", () => {
  it("renders what you said as a bubble, with no avatar", () => {
    const { container } = show(envelope({}));
    expect(screen.getByText("hello there")).toBeTruthy();
    expect(screen.getByText("You")).toBeTruthy();
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

describe("agent-to-agent traffic", () => {
  const peerMessage = envelope({
    channelId: "chef",
    from: { kind: "agent", id: "manager" },
    to: { kind: "agent", id: "chef" },
    hop: 2,
    parts: [{ type: "text", text: "a very long briefing document" }],
  });

  it("collapses to one line instead of a wall of text", () => {
    // This is the whole point: peer chatter must not bury the operator's own
    // conversation.
    const { container } = show(peerMessage);
    expect(screen.getByText(/Received from Manager/)).toBeTruthy();
    expect(screen.queryByText("a very long briefing document")).toBeNull();
    expect(container.querySelector(".msg")).toBeNull();
  });

  it("opens the content in a modal when clicked", () => {
    show(peerMessage);
    fireEvent.click(screen.getByRole("button"));
    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getByText("a very long briefing document")).toBeTruthy();
  });

  it("closes the modal again", () => {
    show(peerMessage);
    fireEvent.click(screen.getByRole("button"));
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("names both ends in the activity feed", () => {
    show(peerMessage, true);
    expect(screen.getByText(/Manager → Chef/)).toBeTruthy();
  });

  it("shows the hop so a relay chain is visible", () => {
    show(peerMessage);
    expect(screen.getByText("hop 2")).toBeTruthy();
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

  it("shows one line per recipient, naming each", () => {
    // "sent to 2 agents" hides which two.
    show(
      record({
        type: "toolCall",
        name: "send_message",
        arguments: { to: ["Chef", "Ghost"], text: "please review" },
        outcome: { status: "ok", summary: "queued for 2 agent(s)" },
      }),
    );
    expect(screen.getByText(/Sent to Chef/)).toBeTruthy();
    expect(screen.getByText(/Sent to Ghost/)).toBeTruthy();
  });

  it("puts the message body behind the click, not in the transcript", () => {
    show(
      record({
        type: "toolCall",
        name: "send_message",
        arguments: { to: ["Chef"], text: "the briefing" },
        outcome: { status: "ok", summary: "queued" },
      }),
    );
    expect(screen.queryByText("the briefing")).toBeNull();
    fireEvent.click(screen.getByRole("button"));
    expect(screen.getByText("the briefing")).toBeTruthy();
  });

  it("says plainly when a message did not go, and why", () => {
    show(
      record({
        type: "toolCall",
        name: "send_message",
        arguments: { to: ["Chef"], text: "undelivered" },
        outcome: { status: "refused", reason: "Refused: hop limit reached." },
      }),
    );
    expect(screen.getByText(/Not delivered to Chef/)).toBeTruthy();
    // On the chip and again in full when opened, which is why this counts
    // rather than asserting a single match.
    expect(screen.getAllByText(/hop limit reached/).length).toBe(1);
    fireEvent.click(screen.getByRole("button"));
    expect(screen.getAllByText(/hop limit reached/).length).toBeGreaterThan(1);
  });

  it("marks only the recipients a half-delivered send actually missed", () => {
    // The bug this replaces: one verdict for the whole call, so a send that
    // reached two of three drew all three as delivered.
    show(
      record({
        type: "toolCall",
        name: "send_message",
        arguments: { to: ["Chef", "Ghost", "Sous"], text: "standup" },
        outcome: {
          status: "partial",
          summary: "queued for 2 of 3 agent(s)",
          refused: [{ to: "Ghost", reason: "Refused: Ghost has been deleted." }],
        },
      }),
    );
    expect(screen.getByText(/Not delivered to Ghost/)).toBeTruthy();
    expect(screen.getByText(/Sent to Chef/)).toBeTruthy();
    expect(screen.getByText(/Sent to Sous/)).toBeTruthy();
    expect(screen.queryByText(/Sent to Ghost/)).toBeNull();
  });

  it("says why a message did not go, without needing a click", () => {
    // A row of bare "not delivered" chips reads as the app breaking. The
    // reason is usually a guard doing its job, and it was behind a click.
    show(
      record({
        type: "toolCall",
        name: "send_message",
        arguments: { to: ["Chef"], text: "the same thing again" },
        outcome: {
          status: "refused",
          reason:
            "Refused: you already sent Chef this exact message in this run. Repeating it will not produce a different reply. Move on.",
        },
      }),
    );
    expect(screen.getByText("you already sent Chef this exact message in this run")).toBeTruthy();
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
    expect(screen.getByText(/checked who is available/)).toBeTruthy();
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
    expect(screen.getByText(/updated its memory/)).toBeTruthy();
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
    expect(screen.getByText(/used run_code/)).toBeTruthy();
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
    render(
      <MessageItem
        message={failure("m-original")}
        lookups={lookups}
        continued={false}
        feed={false}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(retryTurn).toHaveBeenCalledWith("manager", "m-original");
    // And it will not fire twice on a double click: a second run is a second
    // model call, billed.
    expect((screen.getByRole("button", { name: "Sent again" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
  });

  it("offers nothing when there is nothing to send again", () => {
    render(
      <MessageItem message={failure(null)} lookups={lookups} continued={false} feed={false} />,
    );
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
    render(<MessageItem message={stopped} lookups={lookups} continued={false} feed={false} />);
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
      <MessageItem message={used} lookups={lookups} continued={false} feed={false} />,
    );

    expect(container.textContent).toContain("used Mistral ($MISTRAL_API_KEY)");
    expect(container.textContent).toContain("Manager used run_command");
  });
});

describe("redrawing a transcript", () => {
  it("does not draw an entry again when nothing about it changed", () => {
    // A transcript is rebuilt whenever any message is appended, and drawing an
    // entry parses its markdown. Ten agents reporting at once meant every
    // message on screen re-parsed for each arrival, which is the other half of
    // what made the window stop responding.
    const message = envelope({ parts: [{ type: "text", text: "the answer is 42" }] });
    const view = render(
      <MessageItem message={message} lookups={lookups} continued={false} feed={false} />,
    );
    const drawn = screen.getByText("the answer is 42");

    // The same envelope, the same lookups: a parent redraw with nothing new.
    view.rerender(
      <MessageItem message={message} lookups={lookups} continued={false} feed={false} />,
    );

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
        feed={false}
      />,
    );
    view.rerender(
      <MessageItem
        message={envelope({ parts: [{ type: "text", text: "second" }] })}
        lookups={lookups}
        continued={false}
        feed={false}
      />,
    );

    expect(screen.getByText("second")).toBeTruthy();
    expect(screen.queryByText("first")).toBeNull();
  });
});
