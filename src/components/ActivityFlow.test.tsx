import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AgentCard, Envelope } from "../lib/types";
import { ActivityFlow } from "./ActivityFlow";

vi.mock("../lib/ipc", () => ({ openExternal: vi.fn() }));

function card(id: string, name: string, color = "#c7d96b"): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    name,
    avatar: "plain",
    color,
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    version: 1,
    createdAt: 0,
    updatedAt: 0,
  };
}

const AGENTS = [
  card("manager", "Manager"),
  card("critic", "Critic", "#e2674a"),
  card("researcher", "Researcher", "#6aa9d9"),
];
const byId = (id: string) => AGENTS.find((a) => a.id === id);

let clock = 1_700_000_000_000;
function msg(
  from: Envelope["from"],
  to: Envelope["to"],
  text: string,
  hop: number,
  runId = "r1",
): Envelope {
  clock += 1000;
  return {
    id: `m${clock}`,
    runId,
    channelId: to.kind === "agent" ? to.id : "manager",
    from,
    to,
    parts: [{ type: "text", text }],
    trust: "peer",
    hop,
    expectsReply: true,
    cause: null,
    createdAt: clock,
  };
}

const human = { kind: "human" } as const;
const agent = (id: string) => ({ kind: "agent", id }) as const;

/** The exchange from the brief: you, then a relay chain between three agents. */
const CONVERSATION = [
  msg(human, agent("manager"), "find the meaning of life", 0),
  msg(agent("manager"), agent("critic"), "please review", 1),
  msg(agent("critic"), agent("manager"), "here are the holes", 2),
  msg(agent("manager"), agent("researcher"), "go deeper", 3),
  msg(agent("critic"), agent("researcher"), "and check this", 3),
  msg(agent("researcher"), agent("manager"), "findings", 4),
];

describe("ActivityFlow", () => {
  it("gives every participant a lane, the operator included", () => {
    // A flow that starts at the first agent-to-agent message hides who set it
    // off, so "You" has to be on the board.
    render(<ActivityFlow messages={CONVERSATION} byId={byId} />);
    for (const name of ["You", "Manager", "Critic", "Researcher"]) {
      expect(screen.getByText(name)).toBeTruthy();
    }
  });

  it("orders lanes by when each participant joined", () => {
    const { container } = render(<ActivityFlow messages={CONVERSATION} byId={byId} />);
    const names = [...container.querySelectorAll(".flow__name")].map((n) => n.textContent);
    expect(names).toEqual(["You", "Manager", "Critic", "Researcher"]);
  });

  it("draws one arrow per message, in order", () => {
    const { container } = render(<ActivityFlow messages={CONVERSATION} byId={byId} />);
    expect(container.querySelectorAll(".flow__row")).toHaveLength(CONVERSATION.length);
  });

  it("labels each arrow with who sent it and to whom", () => {
    render(<ActivityFlow messages={CONVERSATION} byId={byId} />);
    expect(screen.getByLabelText(/Manager to Critic/)).toBeTruthy();
    expect(screen.getByLabelText(/Critic to Researcher/)).toBeTruthy();
  });

  it("opens the message when an arrow is clicked", () => {
    render(<ActivityFlow messages={CONVERSATION} byId={byId} />);
    fireEvent.click(screen.getByLabelText(/Manager to Critic/));
    const dialog = screen.getByRole("dialog");
    // Scoped to the dialog: the hovered arrow keeps its excerpt behind it.
    expect(within(dialog).getByText("please review")).toBeTruthy();
    expect(within(dialog).getByText(/Manager → Critic/)).toBeTruthy();
  });

  it("is reachable from the keyboard", () => {
    // Each row is a real button, so focus and Enter come from the platform
    // rather than from a tabindex and a key handler on an SVG group.
    render(<ActivityFlow messages={CONVERSATION} byId={byId} />);
    const node = screen.getByLabelText(/Critic to Manager/);
    expect(node.tagName).toBe("BUTTON");
    node.focus();
    expect(document.activeElement).toBe(node);
    fireEvent.click(node);
    expect(within(screen.getByRole("dialog")).getByText("here are the holes")).toBeTruthy();
  });

  it("marks where a new run begins", () => {
    const { container } = render(
      <ActivityFlow
        messages={[
          msg(human, agent("manager"), "first task", 0, "r1"),
          msg(agent("manager"), agent("critic"), "relay", 1, "r1"),
          msg(human, agent("manager"), "second task", 0, "r2"),
        ]}
        byId={byId}
      />,
    );
    // One divider per run, so the board reads as separate errands.
    expect(container.querySelectorAll(".flow__divider")).toHaveLength(2);
  });

  it("shows what was said without needing a click", () => {
    // The whole reason the board was turned upright: an arrow with no room for
    // a word meant reading a conversation one click at a time.
    render(<ActivityFlow messages={CONVERSATION} byId={byId} />);
    expect(screen.getByText(/please review/)).toBeTruthy();
    expect(screen.getByText(/here are the holes/)).toBeTruthy();
  });

  it("invites a first message when nothing has happened", () => {
    render(<ActivityFlow messages={[]} byId={byId} />);
    expect(screen.getByText(/Nothing has happened yet/)).toBeTruthy();
  });

  it("still draws a lane for an agent that has been deleted", () => {
    render(
      <ActivityFlow
        messages={[msg(agent("ghost"), agent("manager"), "from beyond", 1)]}
        byId={byId}
      />,
    );
    expect(screen.getByText("Deleted agent")).toBeTruthy();
  });
});
