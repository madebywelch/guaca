import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AgentCard, Envelope, Part } from "../lib/types";
import { type Lookups, MessageItem } from "./MessageItem";

vi.mock("../lib/ipc", () => ({ openExternal: vi.fn() }));

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

  it("does not draw a note update as a message to nobody", () => {
    // update_notes has no recipients, so falling through to the send renderer
    // drew it as "Sent to no one" with the note body as the message.
    show(
      record({
        type: "toolCall",
        name: "update_notes",
        arguments: { content: "Smith handles verification." },
        outcome: { status: "ok", summary: "Notes saved (28 characters)." },
      }),
    );
    expect(screen.queryByText(/no one/)).toBeNull();
    expect(screen.getByText(/updated its notes/)).toBeTruthy();
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
