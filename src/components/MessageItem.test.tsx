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

function card(id: string, name: string): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-000000000001",
    computerId: null,
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

function show(message: Envelope) {
  return render(<MessageItem message={message} lookups={lookups} continued={false} />);
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
