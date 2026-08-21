import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, Envelope, RunId, RunUsage } from "../lib/types";
import { ActivityFlow } from "./ActivityFlow";

const usageForRuns = vi.fn<(runs: RunId[]) => Promise<RunUsage[]>>(async () => []);

vi.mock("../lib/ipc", () => ({
  openExternal: vi.fn(),
  api: { usageForRuns: (runs: RunId[]) => usageForRuns(runs) },
}));

function card(id: string, name: string, color = "#c7d96b"): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    name,
    avatar: "plain",
    color,
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
    intent: "courtesy",
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

/**
 * Renders the board and lets the usage fetch land. The board asks what each
 * run cost as soon as it mounts, and the answer arrives after the render
 * returns; a test that has already moved on by then is warned about a state
 * update it never waited for.
 */
async function draw(messages: Envelope[], lookup: (id: string) => AgentCard | undefined = byId) {
  const rendered = render(<ActivityFlow messages={messages} byId={lookup} />);
  await act(async () => {});
  return rendered;
}

describe("ActivityFlow", () => {
  beforeEach(() => {
    usageForRuns.mockReset();
    usageForRuns.mockResolvedValue([]);
  });

  it("gives every participant a lane, the operator included", async () => {
    // A flow that starts at the first agent-to-agent message hides who set it
    // off, so "You" has to be on the board.
    await draw(CONVERSATION);
    for (const name of ["You", "Manager", "Critic", "Researcher"]) {
      expect(screen.getByText(name)).toBeTruthy();
    }
  });

  it("orders lanes by when each participant joined", async () => {
    const { container } = await draw(CONVERSATION);
    const names = [...container.querySelectorAll(".flow__name")].map((n) => n.textContent);
    expect(names).toEqual(["You", "Manager", "Critic", "Researcher"]);
  });

  it("draws one arrow per message, in order", async () => {
    const { container } = await draw(CONVERSATION);
    expect(container.querySelectorAll(".flow__row")).toHaveLength(CONVERSATION.length);
  });

  it("labels each arrow with who sent it and to whom", async () => {
    await draw(CONVERSATION);
    expect(screen.getByLabelText(/Manager to Critic/)).toBeTruthy();
    expect(screen.getByLabelText(/Critic to Researcher/)).toBeTruthy();
  });

  it("opens the message when an arrow is clicked", async () => {
    await draw(CONVERSATION);
    fireEvent.click(screen.getByLabelText(/Manager to Critic/));
    const dialog = screen.getByRole("dialog");
    // Scoped to the dialog: the hovered arrow keeps its excerpt behind it.
    expect(within(dialog).getByText("please review")).toBeTruthy();
    expect(within(dialog).getByText(/Manager → Critic/)).toBeTruthy();
  });

  it("is reachable from the keyboard", async () => {
    // Each row is a real button, so focus and Enter come from the platform
    // rather than from a tabindex and a key handler on an SVG group.
    await draw(CONVERSATION);
    const node = screen.getByLabelText(/Critic to Manager/);
    expect(node.tagName).toBe("BUTTON");
    node.focus();
    expect(document.activeElement).toBe(node);
    fireEvent.click(node);
    expect(within(screen.getByRole("dialog")).getByText("here are the holes")).toBeTruthy();
  });

  it("gives each run its own board, newest first", async () => {
    const { container } = await draw([
      msg(human, agent("manager"), "first task", 0, "r1"),
      msg(agent("manager"), agent("critic"), "relay", 1, "r1"),
      msg(human, agent("manager"), "second task", 0, "r2"),
    ]);
    const runs = container.querySelectorAll(".run");
    expect(runs).toHaveLength(2);
    // What just happened is what an operator came here for, so it is at the
    // top and already open; the rest is one line each until asked for.
    expect(runs[0]!.textContent).toContain("second task");
    expect(runs[0]!.getAttribute("data-open")).toBe("true");
    expect(runs[1]!.getAttribute("data-open")).toBeNull();
  });

  it("does not widen a run with participants that were not in it", async () => {
    // Lanes used to be global, so every agent that ever spoke held a column
    // forever and each new one pushed the arrows further right.
    const { container } = await draw([
      msg(human, agent("manager"), "first task", 0, "r1"),
      msg(agent("manager"), agent("critic"), "relay", 1, "r1"),
      msg(human, agent("manager"), "second task", 0, "r2"),
    ]);
    // The open board is the second run: You and Manager, not Critic.
    const lanes = container.querySelectorAll(".run[data-open] .flow__lane");
    expect(lanes).toHaveLength(2);
    expect([...lanes].map((l) => l.textContent)).not.toContain("Critic");
  });

  it("marks an agent that has been deleted, so two of a name are not confused", async () => {
    const gone = {
      id: "ghost",
      name: "Researcher",
      color: "#8aa0a6",
      avatar: "plain",
      lifecycle: "terminated",
    };
    await draw([msg(human, agent("ghost"), "who are you", 0, "r1")], (id) =>
      id === "ghost" ? (gone as never) : byId(id),
    );
    expect(screen.getByText("deleted")).toBeTruthy();
  });

  it("shows what was said without needing a click", async () => {
    // The whole reason the board was turned upright: an arrow with no room for
    // a word meant reading a conversation one click at a time.
    await draw(CONVERSATION);
    expect(screen.getByText(/please review/)).toBeTruthy();
    expect(screen.getByText(/here are the holes/)).toBeTruthy();
  });

  it("invites a first message when nothing has happened", async () => {
    await draw([]);
    expect(screen.getByText(/Nothing has happened yet/)).toBeTruthy();
    expect(usageForRuns).not.toHaveBeenCalled();
  });

  it("still draws a lane for an agent that has been deleted", async () => {
    await draw([msg(agent("ghost"), agent("manager"), "from beyond", 1)]);
    expect(screen.getByText("Deleted agent")).toBeTruthy();
  });

  it("shows what each run cost once the usage arrives", async () => {
    // Asked for once per set of runs, not carried on every message; the answer
    // lands after the board is drawn and the run header fills in.
    usageForRuns.mockResolvedValue([
      { runId: "r1", prompt: 1200, completion: 300, cost: 0.0123, calls: 3 },
    ]);
    await draw(CONVERSATION);
    expect(usageForRuns).toHaveBeenCalledWith(["r1"]);
    expect(screen.getByText("1.5k")).toBeTruthy();
    expect(screen.getByText("$0.012")).toBeTruthy();
    expect(screen.getByTitle(/1,200 in, 300 out, over 3 model call/)).toBeTruthy();
  });
});
