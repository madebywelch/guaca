import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { Activity, AgentCard, Group } from "../lib/types";
import { Sidebar } from "./Sidebar";

const moveAgent =
  vi.fn<(id: string, groupId: string, before: string | null) => Promise<AgentCard>>();
const setAgentPinned = vi.fn<(id: string, pinned: boolean) => Promise<AgentCard>>();

vi.mock("../lib/ipc", () => ({
  api: {
    listAgents: async () => [],
    listGroups: async () => [],
    // Clicking a row opens a channel, and going inside a crew can close one:
    // both read what they are about to draw.
    channelMessages: async () => [],
    conversationFlow: async () => [],
    moveAgent: (id: string, groupId: string, before: string | null) =>
      moveAgent(id, groupId, before),
    setAgentPinned: (id: string, pinned: boolean) => setAgentPinned(id, pinned),
  },
}));

const MODEL = "anthropic/claude-opus-4-1-20250805";
const DEFAULT_GROUP = "00000000-0000-4000-8000-000000000001";

function group(name: string, defaultModel: string | null = null, id = DEFAULT_GROUP): Group {
  return {
    id,
    name,
    agentCount: 0,
    createdAt: 0,
    baseUrl: null,
    defaultModel,
    apiKeySet: false,
    apiKeyHint: "",
  };
}

function agent(name: string, over: Partial<AgentCard> = {}): AgentCard {
  return {
    id: name,
    groupId: DEFAULT_GROUP,
    sandboxId: null,
    browserId: null,
    name,
    avatar: "avocado",
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
    ...over,
  };
}

function draw(groups: Group[], agents: AgentCard[] = [], activity: Record<string, Activity> = {}) {
  useStore.setState({
    agents,
    groups,
    activity,
    lastActive: {},
    usage: Object.fromEntries(
      groups.map((g) => [g.id, { prompt: 900_000, completion: 900_000, calls: 400, cost: 123.45 }]),
    ),
    pulse: {},
    pulses: [],
    selected: null,
    railGroup: null,
  });
  return render(
    <Sidebar
      onNewAgent={vi.fn()}
      onEditAgent={vi.fn()}
      onNewGroup={vi.fn()}
      onEditGroup={vi.fn()}
      onOpenCafeteria={vi.fn()}
      onOpenSettings={vi.fn()}
      onOpenSearch={vi.fn()}
      onOpenMenu={vi.fn()}
    />,
  );
}

/** A row, by the name written in it. */
function row(name: string): HTMLElement {
  const found = screen
    .getAllByRole("button")
    .find((node) => node.className === "agent-row" && node.textContent?.startsWith(name));
  if (!found) throw new Error(`no row for ${name}`);
  return found;
}

/**
 * One drag, from a row to whatever should catch it.
 *
 * Written out rather than wrapped in a helper that fires everything at once,
 * because the press has to travel before it becomes a drag and that threshold is
 * the thing keeping a row a button.
 */
async function dragTo(from: HTMLElement, onto: HTMLElement) {
  fireEvent.pointerDown(from, { button: 0, clientX: 100, clientY: 200 });
  fireEvent.pointerMove(window, { clientX: 100, clientY: 240 });
  fireEvent.pointerEnter(onto, { clientX: 100, clientY: 260 });
  fireEvent.pointerUp(window, { clientX: 100, clientY: 260 });
  // The drop is a command and a re-read, so let both settle.
  await vi.waitFor(() =>
    expect(moveAgent.mock.calls.length + setAgentPinned.mock.calls.length).toBeGreaterThan(0),
  );
}

beforeEach(() => {
  moveAgent.mockReset();
  moveAgent.mockResolvedValue(agent("Manager"));
  setAgentPinned.mockReset();
  setAgentPinned.mockResolvedValue(agent("Manager"));
  useStore.setState({ groups: [], agents: [], railGroup: null });
});

describe("group header", () => {
  it("does not draw the pinned model in the rail at all", () => {
    // The rail is 15.5rem. A model id beside the name left one letter of
    // "everyone" and an ellipsis; on a line of its own it cost a whole row per
    // group to say something the gear already shows.
    const { container } = draw([group("everyone", MODEL)]);

    expect(container.querySelector(".rail__group-head")?.textContent).toContain("everyone");
    expect(screen.queryByText(MODEL)).toBeNull();
  });
});

describe("arranging the rail", () => {
  it("drops a row in front of the one it stopped on coming up", async () => {
    draw(
      [group("everyone")],
      [
        agent("Manager", { railOrder: 0 }),
        agent("Cook", { railOrder: 1 }),
        agent("Scribe", { railOrder: 2 }),
      ],
    );

    await dragTo(row("Scribe"), row("Manager"));

    expect(moveAgent).toHaveBeenCalledWith("Scribe", DEFAULT_GROUP, "Manager");
  });

  it("drops a row after the one it passed going down", async () => {
    draw(
      [group("everyone")],
      [
        agent("Manager", { railOrder: 0 }),
        agent("Cook", { railOrder: 1 }),
        agent("Scribe", { railOrder: 2 }),
      ],
    );

    await dragTo(row("Manager"), row("Cook"));

    expect(moveAgent).toHaveBeenCalledWith("Manager", DEFAULT_GROUP, "Scribe");
  });

  it("marks the row a release would land in front of, and the row in hand", () => {
    // The only two things on screen saying what the gesture will do. A drag
    // that shows neither is a drag the operator has to guess the result of.
    draw(
      [group("everyone")],
      [agent("Manager", { railOrder: 0 }), agent("Cook", { railOrder: 1 })],
    );

    fireEvent.pointerDown(row("Cook"), { button: 0, clientX: 100, clientY: 300 });
    fireEvent.pointerMove(window, { clientX: 100, clientY: 250 });
    fireEvent.pointerEnter(row("Manager"), { clientX: 100, clientY: 240 });

    expect(row("Manager").dataset.over).toBe("true");
    expect(row("Cook").dataset.held).toBe("true");
    // Not on the row being carried, which is not somewhere it can land.
    expect(row("Cook").dataset.over).toBeUndefined();
  });

  it("draws the arrangement while a drag is on, not whoever is mid-turn", async () => {
    // Dragging is arranging, so it has to operate on the arrangement. A row
    // dropped under a peer that is only near the top because it is working
    // would land somewhere the operator never aimed at.
    draw(
      [group("everyone")],
      [
        agent("Manager", { railOrder: 0 }),
        agent("Cook", { railOrder: 1 }),
        agent("Scribe", { railOrder: 2 }),
      ],
      { Scribe: { state: "thinking" } },
    );

    const names = () =>
      [...document.querySelectorAll(".agent-row__name")].map((n) => n.textContent);
    expect(names()).toEqual(["Scribe", "Manager", "Cook"]);

    fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 100, clientY: 300 });
    fireEvent.pointerMove(window, { clientX: 100, clientY: 250 });
    expect(names()).toEqual(["Manager", "Cook", "Scribe"]);
  });

  it("leaves the rail alone when a press does not travel", () => {
    // A row is a button first. Selecting an agent with a hand that is not
    // perfectly still must not start rearranging anything.
    draw([group("everyone")], [agent("Manager"), agent("Cook", { railOrder: 1 })]);

    fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 100, clientY: 200 });
    fireEvent.pointerEnter(row("Cook"), { clientX: 100, clientY: 202 });
    fireEvent.pointerUp(window, { clientX: 100, clientY: 202 });

    expect(moveAgent).not.toHaveBeenCalled();
  });

  it("abandons a drag on escape without moving anything", () => {
    draw([group("everyone")], [agent("Manager"), agent("Cook", { railOrder: 1 })]);

    fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 100, clientY: 200 });
    fireEvent.pointerMove(window, { clientX: 100, clientY: 240 });
    fireEvent.pointerEnter(row("Cook"), { clientX: 100, clientY: 260 });
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.pointerUp(window, { clientX: 100, clientY: 260 });

    expect(moveAgent).not.toHaveBeenCalled();
  });

  it("pins a row dropped on the pinned section and moves it nowhere", async () => {
    // The section spans groups, so there is no place in it to express. Pinning
    // is the whole of what the gesture asked for.
    draw(
      [group("everyone")],
      [agent("Manager", { railOrder: 0, pinned: true }), agent("Cook", { railOrder: 1 })],
    );

    const pinned = document.querySelector(".rail__group");
    if (!pinned) throw new Error("the pinned section was not drawn");

    fireEvent.pointerDown(row("Cook"), { button: 0, clientX: 100, clientY: 300 });
    fireEvent.pointerMove(window, { clientX: 100, clientY: 250 });
    fireEvent.pointerEnter(pinned, { clientX: 100, clientY: 210 });
    fireEvent.pointerUp(window, { clientX: 100, clientY: 210 });

    await vi.waitFor(() => expect(setAgentPinned).toHaveBeenCalledWith("Cook", true));
    expect(moveAgent).not.toHaveBeenCalled();
  });

  it("unpins a row dragged out of the pins and into a crew", async () => {
    draw(
      [group("everyone")],
      [
        agent("Manager", { railOrder: 0, pinned: true }),
        agent("Cook", { railOrder: 1 }),
        agent("Scribe", { railOrder: 2 }),
      ],
    );

    await dragTo(row("Manager"), row("Scribe"));

    // In front of Scribe, not at the end: an agent arriving from another
    // section has no place in this one to have travelled from, so there is no
    // direction to read and it takes the place of what it was dropped on.
    expect(setAgentPinned).toHaveBeenCalledWith("Manager", false);
    expect(moveAgent).toHaveBeenCalledWith("Manager", DEFAULT_GROUP, "Scribe");
  });
});

describe("groups as places", () => {
  const RESEARCH = "00000000-0000-4000-8000-000000000002";

  it("keeps the strip out of the way while there is one group", () => {
    draw([group("everyone")], [agent("Manager")]);
    expect(screen.queryByLabelText("Groups")).toBeNull();
  });

  it("draws one circle per group, and the faces in it", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );

    expect(screen.getByLabelText("Groups")).toBeTruthy();
    expect(screen.getByLabelText("research, 1 agent")).toBeTruthy();
    expect(screen.getByTitle("Reader")).toBeTruthy();
  });

  it("says on the circle when somebody inside it needs the operator", () => {
    // After focusing on one group the strip is the only place the other crews
    // are still visible, so it has to carry the one state that is waiting on a
    // person.
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
      { Reader: { state: "awaitingApproval" } },
    );

    expect(screen.getByLabelText("research, 1 agent, someone needs you")).toBeTruthy();
  });

  it("draws only that group after clicking into it, and everyone again after leaving", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );

    fireEvent.click(screen.getByLabelText("research, 1 agent"));
    expect(screen.getByText("Reader")).toBeTruthy();
    expect(screen.queryByText("Manager")).toBeNull();

    fireEvent.click(screen.getByLabelText("All groups, 2 agents"));
    expect(screen.getByText("Manager")).toBeTruthy();
  });

  it("closes a channel from the crew being left, and keeps one from the crew opened", async () => {
    // Two crews can hold two agents with the same name and the same face, and
    // going inside one does not draw the other's row: a channel left open from
    // the crew you came from reads as this crew's, working while nobody here is.
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Chief"), agent("Chief of research", { groupId: RESEARCH, railOrder: 1 })],
    );
    useStore.setState({ selected: "Chief" });

    fireEvent.click(screen.getByLabelText("research, 1 agent"));
    await vi.waitFor(() => expect(useStore.getState().selected).toBe("activity"));

    fireEvent.click(screen.getByLabelText("All groups, 2 agents"));
    fireEvent.click(row("Chief of research"));
    fireEvent.click(screen.getByLabelText("research, 1 agent"));
    await vi.waitFor(() => expect(useStore.getState().railGroup).toBe(RESEARCH));
    expect(useStore.getState().selected).toBe("Chief of research");
  });

  it("moves an agent into the group whose circle it was dropped on", async () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );

    fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 100, clientY: 300 });
    fireEvent.pointerMove(window, { clientX: 100, clientY: 250 });
    fireEvent.pointerEnter(screen.getByLabelText("research, 1 agent"), {
      clientX: 60,
      clientY: 120,
    });
    fireEvent.pointerUp(window, { clientX: 60, clientY: 120 });

    await vi.waitFor(() => expect(moveAgent).toHaveBeenCalledWith("Manager", RESEARCH, null));
  });
});
