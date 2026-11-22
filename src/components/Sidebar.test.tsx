import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { Activity, AgentCard, Escalation, Group } from "../lib/types";
import { aGroup, DEFAULT_GROUP } from "../test-fixtures";
import { Sidebar } from "./Sidebar";

const moveAgent =
  vi.fn<(id: string, groupId: string, before: string | null) => Promise<AgentCard>>();
const setAgentPinned = vi.fn<(id: string, pinned: boolean) => Promise<AgentCard>>();
const moveGroup = vi.fn<(id: string, before: string | null) => Promise<Group[]>>();

vi.mock("../lib/ipc", () => ({
  api: {
    listAgents: async () => [],
    listGroups: async () => [],
    channelMessages: async () => [],
    conversationFlow: async () => [],
    moveAgent: (id: string, groupId: string, before: string | null) =>
      moveAgent(id, groupId, before),
    setAgentPinned: (id: string, pinned: boolean) => setAgentPinned(id, pinned),
    moveGroup: (id: string, before: string | null) => moveGroup(id, before),
  },
}));

const MODEL = "anthropic/claude-opus-4-1-20250805";
function group(name: string, defaultModel: string | null = null, id = DEFAULT_GROUP): Group {
  return aGroup({ id, name, inference: { ...aGroup().inference, defaultModel } });
}

function agent(name: string, over: Partial<AgentCard> = {}): AgentCard {
  return {
    id: name,
    groupId: DEFAULT_GROUP,
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    readsWeb: true,
    browserConsent: "open",
    name,
    avatar: "avocado",
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
    ...over,
  };
}

function draw(
  groups: Group[],
  agents: AgentCard[] = [],
  activity: Record<string, Activity> = {},
  stuck: Escalation[] = [],
) {
  useStore.setState({
    agents,
    groups,
    activity,
    stuck,
    lastActive: {},
    usage: Object.fromEntries(
      groups.map((g) => [g.id, { prompt: 900_000, completion: 900_000, calls: 400, cost: 123.45 }]),
    ),
    pulse: {},
    pulses: [],
    selected: null,
    railGroup: null,
  });
  const onNewAgent = vi.fn();
  const onNewGroup = vi.fn();
  const onOpenCafeteria = vi.fn();
  return {
    ...render(
      <Sidebar
        onEditAgent={vi.fn()}
        onEditGroup={vi.fn()}
        onOpenCafeteria={onOpenCafeteria}
        onOpenCalendar={vi.fn()}
        onOpenArtifacts={vi.fn()}
        onOpenSettings={vi.fn()}
        onOpenCommandMenu={vi.fn()}
        onNewAgent={onNewAgent}
        onNewGroup={onNewGroup}
        onOpenMenu={vi.fn()}
      />,
    ),
    onNewAgent,
    onNewGroup,
    onOpenCafeteria,
  };
}

function row(name: string): HTMLElement {
  const found = screen
    .getAllByRole("button")
    .find((node) => node.className === "agent-row" && node.textContent?.startsWith(name));
  if (!found) throw new Error(`no row for ${name}`);
  return found;
}

function names(): (string | null)[] {
  return [...document.querySelectorAll(".agent-row__name")].map((n) => n.textContent);
}

async function dragTo(from: HTMLElement, onto: HTMLElement) {
  fireEvent.pointerDown(from, { button: 0, clientX: 100, clientY: 200 });
  fireEvent.pointerMove(window, { clientX: 100, clientY: 240 });
  fireEvent.pointerEnter(onto, { clientX: 100, clientY: 260 });
  fireEvent.pointerUp(window, { clientX: 100, clientY: 260 });
  await vi.waitFor(() =>
    expect(moveAgent.mock.calls.length + setAgentPinned.mock.calls.length).toBeGreaterThan(0),
  );
}

beforeEach(() => {
  moveAgent.mockReset();
  moveAgent.mockResolvedValue(agent("Manager"));
  setAgentPinned.mockReset();
  setAgentPinned.mockResolvedValue(agent("Manager"));
  moveGroup.mockReset();
  moveGroup.mockImplementation(async () => useStore.getState().groups);
  useStore.setState({ groups: [], agents: [], railGroup: null });
});

describe("group header", () => {
  it("does not draw the pinned model in the rail at all", () => {
    const { container } = draw([group("everyone", MODEL)]);

    expect(container.querySelector(".rail__group-head")?.textContent).toContain("everyone");
    expect(screen.queryByText(MODEL)).toBeNull();
  });
});

describe("the plus", () => {
  it("is drawn with nothing in the workspace, which is when it is the only thing to do", () => {
    draw([], []);

    expect(screen.getByRole("button", { name: /make something new/i })).toBeTruthy();
  });

  it("makes a group from the rail, which is the list a group is a heading in", () => {
    const { onNewGroup } = draw([group("everyone")], [agent("Manager")]);

    fireEvent.click(screen.getByRole("button", { name: /make something new/i }));
    fireEvent.click(screen.getByRole("menuitem", { name: /new group/i }));

    expect(onNewGroup).toHaveBeenCalledOnce();
  });
});

describe("arranging the rail", () => {
  it("starts at the slop boundary and cancels without a drop", () => {
    draw([group("everyone")], [agent("Manager"), agent("Cook", { railOrder: 1 })]);
    fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 100, clientY: 200 });
    fireEvent.pointerMove(window, { clientX: 102, clientY: 202 });
    expect(row("Manager").dataset.held).toBeUndefined();
    fireEvent.pointerMove(window, { clientX: 103, clientY: 202 });
    expect(row("Manager").dataset.held).toBe("true");
    fireEvent.pointerEnter(row("Cook"));
    fireEvent.pointerCancel(window);
    fireEvent.pointerUp(window);
    expect(row("Manager").dataset.held).toBeUndefined();
    expect(moveAgent).not.toHaveBeenCalled();
  });

  it("does not persist a drag released without a target", () => {
    draw([group("everyone")], [agent("Manager")]);
    fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 100, clientY: 200 });
    fireEvent.pointerMove(window, { clientX: 100, clientY: 210 });
    fireEvent.pointerUp(window);
    expect(row("Manager").dataset.held).toBeUndefined();
    expect(moveAgent).not.toHaveBeenCalled();
  });

  it("ends the gesture before a failed drop settles", async () => {
    let rejectDrop!: (reason: Error) => void;
    const pending = new Promise<void>((_, reject) => {
      rejectDrop = reject;
    });
    void pending.catch(() => {});
    const original = useStore.getState().dropAgent;
    const drop = vi.fn(() => pending);
    useStore.setState({ dropAgent: drop });
    try {
      draw([group("everyone")], [agent("Manager"), agent("Cook", { railOrder: 1 })]);
      fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 100, clientY: 200 });
      fireEvent.pointerMove(window, { clientX: 100, clientY: 210 });
      fireEvent.pointerEnter(row("Cook"));
      fireEvent.pointerUp(window);
      expect(drop).toHaveBeenCalledOnce();
      expect(row("Manager").dataset.held).toBeUndefined();
      await act(async () => rejectDrop(new Error("drop failed")));
      expect(row("Manager").dataset.held).toBeUndefined();
    } finally {
      useStore.setState({ dropAgent: original });
    }
  });
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
    draw(
      [group("everyone")],
      [agent("Manager", { railOrder: 0 }), agent("Cook", { railOrder: 1 })],
    );

    fireEvent.pointerDown(row("Cook"), { button: 0, clientX: 100, clientY: 300 });
    fireEvent.pointerMove(window, { clientX: 100, clientY: 250 });
    fireEvent.pointerEnter(row("Manager"), { clientX: 100, clientY: 240 });

    expect(row("Manager").dataset.over).toBe("true");
    expect(row("Cook").dataset.held).toBe("true");
    expect(row("Cook").dataset.over).toBeUndefined();
  });

  it("draws the arrangement while a drag is on, not whoever is mid-turn", async () => {
    draw(
      [group("everyone")],
      [
        agent("Manager", { railOrder: 0 }),
        agent("Cook", { railOrder: 1 }),
        agent("Scribe", { railOrder: 2 }),
      ],
      { Scribe: { state: "thinking" } },
    );

    expect(names()).toEqual(["Scribe", "Manager", "Cook"]);

    fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 100, clientY: 300 });
    fireEvent.pointerMove(window, { clientX: 100, clientY: 250 });
    expect(names()).toEqual(["Manager", "Cook", "Scribe"]);
  });

  it("leaves the rail alone when a press does not travel", () => {
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

  it("pins a row dropped on a pinned peer, and lands it among the pins", async () => {
    draw(
      [group("everyone")],
      [
        agent("Manager", { railOrder: 0, pinned: true }),
        agent("Chef", { railOrder: 1, pinned: true }),
        agent("Cook", { railOrder: 2 }),
      ],
    );

    await dragTo(row("Cook"), row("Chef"));

    await vi.waitFor(() => expect(setAgentPinned).toHaveBeenCalledWith("Cook", true));
    expect(moveAgent).toHaveBeenCalledWith("Cook", DEFAULT_GROUP, "Chef");
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

    expect(setAgentPinned).toHaveBeenCalledWith("Manager", false);
    expect(moveAgent).toHaveBeenCalledWith("Manager", DEFAULT_GROUP, "Scribe");
  });
});

describe("arranging the crews", () => {
  const RESEARCH = "00000000-0000-4000-8000-000000000002";
  const KITCHEN = "00000000-0000-4000-8000-000000000003";

  function column() {
    draw(
      [group("everyone"), group("research", null, RESEARCH), group("kitchen", null, KITCHEN)],
      [agent("Manager")],
    );
  }

  function circle(name: string): HTMLElement {
    return screen.getByLabelText(new RegExp(`^${name}, `));
  }

  function circles(): string[] {
    return [...document.querySelectorAll(".grail__list .orb")].map(
      (node) => node.getAttribute("title") ?? "",
    );
  }

  function carry(from: HTMLElement, onto: HTMLElement) {
    fireEvent.pointerDown(from, { button: 0, clientX: 30, clientY: 200 });
    fireEvent.pointerMove(window, { clientX: 30, clientY: 240 });
    fireEvent.pointerEnter(onto, { clientX: 30, clientY: 260 });
  }

  it("moves nothing when a crew is let go over the rail", () => {
    column();

    carry(circle("kitchen"), row("Manager"));
    fireEvent.pointerEnter(document.querySelector(".rail__group")!, { clientX: 300, clientY: 260 });
    fireEvent.pointerUp(window, { clientX: 300, clientY: 260 });

    expect(moveGroup).not.toHaveBeenCalled();
    expect(moveAgent).not.toHaveBeenCalled();
    expect(row("Manager").dataset.over).toBeUndefined();
  });

  it("leaves the column alone when a press on a circle does not travel", () => {
    column();

    fireEvent.pointerDown(circle("kitchen"), { button: 0, clientX: 30, clientY: 200 });
    fireEvent.pointerEnter(circle("research"), { clientX: 30, clientY: 202 });
    fireEvent.pointerUp(window, { clientX: 30, clientY: 202 });

    expect(moveGroup).not.toHaveBeenCalled();
  });

  it("abandons a drag on escape without moving anything", () => {
    column();

    carry(circle("kitchen"), circle("everyone"));
    fireEvent.keyDown(window, { key: "Escape" });
    fireEvent.pointerUp(window, { clientX: 30, clientY: 260 });

    expect(moveGroup).not.toHaveBeenCalled();
  });

  it("asks for nothing when a crew is dropped back on itself", () => {
    column();

    carry(circle("research"), circle("research"));
    fireEvent.pointerUp(window, { clientX: 30, clientY: 260 });

    expect(moveGroup).not.toHaveBeenCalled();
  });

  it("drops a crew in front of the one it stopped on coming up", async () => {
    column();

    carry(circle("kitchen"), circle("everyone"));
    fireEvent.pointerUp(window, { clientX: 30, clientY: 260 });

    await vi.waitFor(() => expect(moveGroup).toHaveBeenCalledWith(KITCHEN, DEFAULT_GROUP));
  });

  it("drops a crew after the one it passed going down", async () => {
    column();

    carry(circle("everyone"), circle("research"));
    fireEvent.pointerUp(window, { clientX: 30, clientY: 260 });

    await vi.waitFor(() => expect(moveGroup).toHaveBeenCalledWith(DEFAULT_GROUP, KITCHEN));
  });

  it("marks the circle in hand, and which side of the target it would land", () => {
    column();

    carry(circle("kitchen"), circle("research"));
    expect(circle("kitchen").dataset.held).toBe("true");
    expect(circle("research").dataset.lands).toBe("before");
    expect(circle("research").dataset.over).toBeUndefined();
    expect(document.querySelector(".rail__group[data-over]")).toBeNull();
    expect(document.querySelector(".rail__held")).toBeNull();

    fireEvent.keyDown(window, { key: "Escape" });
    carry(circle("everyone"), circle("research"));
    expect(circle("research").dataset.lands).toBe("after");
  });

  it("draws the order the runtime answers with, everywhere crews are listed", async () => {
    column();
    const answered = [
      group("kitchen", null, KITCHEN),
      group("everyone"),
      group("research", null, RESEARCH),
    ];
    moveGroup.mockResolvedValue(answered);

    carry(circle("kitchen"), circle("everyone"));
    fireEvent.pointerUp(window, { clientX: 30, clientY: 260 });

    await vi.waitFor(() => expect(circles()).toEqual(["kitchen", "everyone", "research"]));
    expect(
      [...document.querySelectorAll(".rail__group-name")].map((node) => node.textContent),
    ).toEqual(["kitchen", "everyone", "research"]);
  });
});

describe("pins", () => {
  const RESEARCH = "00000000-0000-4000-8000-000000000002";

  function crews() {
    return [group("everyone"), group("research", null, RESEARCH)];
  }

  function roster() {
    return [
      agent("Manager", { railOrder: 0 }),
      agent("Cook", { railOrder: 1 }),
      agent("Chef", { railOrder: 2, pinned: true }),
      agent("Reader", { groupId: RESEARCH, railOrder: 3 }),
    ];
  }

  it("heads the crew with its pins, in the overview and inside the crew", () => {
    draw(crews(), roster());

    expect(names()).toEqual(["Chef", "Manager", "Cook", "Reader"]);
    expect(screen.queryByText("Pinned")).toBeNull();

    fireEvent.click(screen.getByLabelText("everyone, 3 agents"));
    expect(names()).toEqual(["Chef", "Manager", "Cook"]);
  });

  it("marks the pinned row, because being first is not a state", () => {
    draw(crews(), roster());

    expect(row("Chef").querySelector(".agent-row__pin")).toBeTruthy();
    expect(row("Manager").querySelector(".agent-row__pin")).toBeNull();
  });

  it("keeps the pin when the agent is moved to another crew", async () => {
    draw(crews(), roster());

    await dragTo(row("Chef"), screen.getByLabelText("research, 1 agent"));

    expect(moveAgent).toHaveBeenCalledWith("Chef", RESEARCH, null);
    expect(setAgentPinned).not.toHaveBeenCalled();
  });
});

describe("what the rail offers", () => {
  it("does not offer the flow board", () => {
    draw([group("everyone")], [agent("Manager")]);
    expect(screen.queryByRole("button", { name: /activity/i })).toBeNull();
  });

  it("keeps four places in its footer, and hires from the plus", () => {
    const { container, onOpenCafeteria } = draw([group("everyone")], [agent("Manager")]);
    const foot = container.querySelector<HTMLElement>(".rail__foot");
    if (!foot) throw new Error("no footer");

    const places = within(foot).getAllByRole("button");
    expect(places).toHaveLength(4);
    expect(places).toEqual(
      ["For you", "Calendar", "Artifacts", "App settings"].map((name) =>
        within(foot).getByRole("button", { name: new RegExp(`^${name}`) }),
      ),
    );
    expect(within(foot).queryByRole("button", { name: /cafeteria/i })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /make something new/i }));
    fireEvent.click(screen.getByRole("menuitem", { name: /cafeteria/i }));
    expect(onOpenCafeteria).toHaveBeenCalledOnce();
  });

  it("marks the calendar with the day it is", () => {
    vi.useFakeTimers({ toFake: ["Date"], now: new Date(2026, 8, 3, 12) });
    try {
      const { container } = draw([group("everyone")], [agent("Manager")]);
      const calendar = within(container.querySelector(".rail__foot") as HTMLElement).getByRole(
        "button",
        { name: /Calendar/ },
      );
      expect(calendar.querySelector("b")?.textContent).toBe("Thu");
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("groups as places", () => {
  const RESEARCH = "00000000-0000-4000-8000-000000000002";

  function laid(out: boolean) {
    const box = (over: Partial<DOMRect>) => () => ({ top: 0, right: 0, ...over }) as DOMRect;
    document.querySelector<HTMLElement>(".grail__reach")!.getBoundingClientRect = box({
      top: 36,
      right: 20,
    });
    document.querySelector<HTMLElement>(".grail__slab")!.getBoundingClientRect = box({
      right: out ? 64 : 8,
    });
  }

  function out(): boolean {
    return screen.getByLabelText("Groups").getAttribute("data-out") === "true";
  }

  it("keeps the crews in until the pointer comes at them", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );
    laid(false);

    fireEvent.pointerMove(window, { clientX: 700, clientY: 400 });
    expect(out()).toBe(false);

    fireEvent.pointerMove(window, { clientX: 6, clientY: 400 });
    expect(out()).toBe(true);
  });

  it("leaves the window's own buttons alone", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );
    laid(false);

    fireEvent.pointerMove(window, { clientX: 6, clientY: 18 });
    expect(out()).toBe(false);
  });

  it("stays in for a drag that never goes near it", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );
    laid(false);

    fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 400, clientY: 200 });
    fireEvent.pointerMove(window, { clientX: 400, clientY: 240 });

    expect(out()).toBe(false);
  });

  it("stays out for the rest of a drag that has reached it", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );
    laid(false);

    fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 400, clientY: 200 });
    fireEvent.pointerMove(window, { clientX: 400, clientY: 240 });
    fireEvent.pointerMove(window, { clientX: 6, clientY: 240 });
    expect(out()).toBe(true);

    laid(true);
    fireEvent.pointerMove(window, { clientX: 400, clientY: 240 });
    expect(out()).toBe(true);

    fireEvent.pointerUp(window, { clientX: 400, clientY: 240 });
    fireEvent.pointerMove(window, { clientX: 400, clientY: 240 });
    expect(out()).toBe(false);
  });

  it("keeps the column out of the way while there is one group", () => {
    draw([group("everyone")], [agent("Manager")]);
    expect(screen.queryByLabelText("Groups")).toBeNull();
  });

  it("puts the current group's own face in the lip, and nothing while the rail is outside them", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1, color: "#1c7a51" })],
    );
    const face = () => document.querySelector<HTMLElement>(".grail__mark .avatar");
    expect(face()).toBeNull();

    fireEvent.click(screen.getByLabelText("research, 1 agent"));
    expect(face()?.style.getPropertyValue("--accent")).toBe("#1c7a51");
    expect(document.querySelector(".grail__mark")?.getAttribute("aria-hidden")).toBe("true");
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

  it("seats six of a crew, and counts whoever is past that", () => {
    const cooks = Array.from({ length: 9 }, (_, i) =>
      agent(`Cook ${i}`, { groupId: RESEARCH, railOrder: i }),
    );
    draw([group("everyone"), group("research", null, RESEARCH)], [agent("Manager"), ...cooks]);

    const orb = screen.getByLabelText("research, 9 agents");
    expect(orb.querySelectorAll(".orb__face")).toHaveLength(6);
    expect(orb.textContent).toContain("+3");
  });

  it("says on the circle when somebody inside it needs the operator", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
      { Reader: { state: "awaitingApproval" } },
    );

    const orb = screen.getByLabelText("research, 1 agent, 1 question waiting on you");
    expect(orb.querySelector(".orb__waiting")?.textContent).toBe("1");
  });

  it("counts the questions rather than reporting that there are some", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [
        agent("Manager"),
        agent("Reader", { groupId: RESEARCH, railOrder: 1 }),
        agent("Writer", { groupId: RESEARCH, railOrder: 2 }),
      ],
      { Reader: { state: "awaitingApproval" }, Writer: { state: "awaitingApproval" } },
    );

    const orb = screen.getByLabelText("research, 2 agents, 2 questions waiting on you");
    expect(orb.querySelector(".orb__waiting")?.textContent).toBe("2");
  });

  it("draws no count on a crew that is merely busy", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
      { Reader: { state: "thinking" } },
    );

    const orb = screen.getByLabelText("research, 1 agent, working");
    expect(orb.querySelector(".orb__waiting")).toBeNull();
    expect(orb.dataset.state).toBe("working");
  });

  it("names a crew beside its circle while the circle is pointed at", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );

    const orb = screen.getByLabelText("research, 1 agent");
    expect(orb.querySelector(".orb__tag")).toBeNull();

    fireEvent.pointerEnter(orb);
    expect(orb.querySelector(".orb__tag-name")?.textContent).toBe("research");

    fireEvent.pointerLeave(orb);
    expect(orb.querySelector(".orb__tag")).toBeNull();
  });

  it("names it on a keyboard focus too, and lets go on blur", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );

    const orb = screen.getByLabelText("research, 1 agent");
    fireEvent.focus(orb);
    expect(orb.querySelector(".orb__tag-name")?.textContent).toBe("research");

    fireEvent.blur(orb);
    expect(orb.querySelector(".orb__tag")).toBeNull();
  });

  it("draws the whole name, however long it is", () => {
    const long = "Customer research and competitive intelligence, EMEA";
    draw([group("everyone"), group(long, null, RESEARCH)], [agent("Manager")]);

    const orb = screen.getByLabelText(`${long}, 0 agents`);
    fireEvent.pointerEnter(orb);
    expect(orb.querySelector(".orb__tag-name")?.textContent).toBe(long);
  });

  it("says what the crew is doing under its name, in the label's own words", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
      { Reader: { state: "awaitingApproval" } },
    );

    const orb = screen.getByLabelText("research, 1 agent, 1 question waiting on you");
    fireEvent.pointerEnter(orb);
    expect(orb.querySelector(".orb__tag-note")?.textContent).toBe(
      "1 agent, 1 question waiting on you",
    );
  });

  it("keeps the tag out of the accessibility tree", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );

    const orb = screen.getByLabelText("research, 1 agent");
    fireEvent.pointerEnter(orb);
    expect(orb.querySelector(".orb__tag")?.getAttribute("aria-hidden")).toBe("true");
  });

  it("names the everybody circle as well", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );

    const all = screen.getByLabelText("All groups, 2 agents");
    fireEvent.pointerEnter(all);
    expect(all.querySelector(".orb__tag-name")?.textContent).toBe("All groups");
    expect(all.querySelector(".orb__tag-note")?.textContent).toBe("2 agents");
  });

  it("names the crew a dragged agent is being aimed at", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );

    fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 100, clientY: 300 });
    fireEvent.pointerMove(window, { clientX: 100, clientY: 250 });

    const orb = screen.getByLabelText("research, 1 agent");
    fireEvent.pointerEnter(orb, { clientX: 60, clientY: 200 });

    expect(orb.dataset.over).toBe("true");
    expect(orb.querySelector(".orb__tag-name")?.textContent).toBe("research");
  });

  it("stays inside the crew whose circle is clicked twice", () => {
    draw([group("everyone"), group("research", null, RESEARCH)], [agent("Manager")]);

    fireEvent.click(screen.getByLabelText("research, 0 agents"));
    expect(useStore.getState().railGroup).toBe(RESEARCH);

    fireEvent.click(screen.getByLabelText("research, 0 agents"));
    expect(useStore.getState().railGroup).toBe(RESEARCH);
  });

  it("goes back out to everybody by the circle at the top of the column", () => {
    draw([group("everyone"), group("research", null, RESEARCH)], [agent("Manager")]);

    fireEvent.click(screen.getByLabelText("research, 0 agents"));
    fireEvent.click(screen.getByLabelText("All groups, 1 agents"));

    expect(useStore.getState().railGroup).toBeNull();
  });

  it("marks the crew the rail is inside, and only that one", () => {
    draw([group("everyone"), group("research", null, RESEARCH)], [agent("Manager")]);

    const current = () =>
      [...document.querySelectorAll<HTMLElement>('.orb[aria-current="true"]')].map((orb) =>
        orb.getAttribute("aria-label"),
      );

    expect(current()).toEqual(["All groups, 1 agents"]);

    fireEvent.click(screen.getByLabelText("research, 0 agents"));
    expect(current()).toEqual(["research, 0 agents"]);
  });

  it("keeps the whole name on the heading the rail cuts short", () => {
    const long = "Customer research and competitive intelligence, EMEA";
    draw([group("everyone"), group(long, null, RESEARCH)], [agent("Manager")]);

    expect(document.querySelector(".rail__group-name")?.getAttribute("title")).toBe("everyone");

    fireEvent.click(screen.getByLabelText(`${long}, 0 agents`));
    expect(document.querySelector(".rail__open-name")?.getAttribute("title")).toBe(long);
  });

  it("keeps the crew's spend and its count off the line the name is on", () => {
    draw([group("everyone"), group("StopTheScam", null, RESEARCH)], [agent("Manager")]);
    fireEvent.click(screen.getByLabelText("StopTheScam, 0 agents"));

    const head = document.querySelector(".rail__open-head");
    expect(head?.textContent).toBe("StopTheScam");
    expect(document.querySelector(".spend")).toBeNull();
  });

  it("opens the spend card on a heading held, and not on one passed over", () => {
    vi.useFakeTimers();
    try {
      draw([group("everyone")], [agent("Manager")]);
      const head = document.querySelector(".rail__group-head") as HTMLElement;

      fireEvent.pointerEnter(head);
      act(() => void vi.advanceTimersByTime(120));
      expect(document.querySelector(".spend")).toBeNull();

      act(() => void vi.advanceTimersByTime(400));
      expect(document.querySelector(".spend__total")?.textContent).toBe("1.8M");

      fireEvent.pointerLeave(head);
      expect(document.querySelector(".spend")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("keeps the card shut while an agent is being dragged", () => {
    vi.useFakeTimers();
    try {
      draw([group("everyone")], [agent("Manager"), agent("Reader", { railOrder: 1 })]);
      const head = document.querySelector(".rail__group-head") as HTMLElement;

      fireEvent.pointerEnter(head);
      act(() => void vi.advanceTimersByTime(400));
      expect(document.querySelector(".spend")).toBeTruthy();

      fireEvent.pointerDown(row("Manager"), { button: 0, clientX: 100, clientY: 300 });
      fireEvent.pointerMove(window, { clientX: 100, clientY: 250 });
      expect(document.querySelector(".spend")).toBeNull();
    } finally {
      vi.useRealTimers();
    }
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

  it("keeps the conversation open when browsing groups and changes it when choosing an agent", async () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Chief"), agent("Chief of research", { groupId: RESEARCH, railOrder: 1 })],
    );
    useStore.setState({ selected: "Chief" });

    fireEvent.click(screen.getByLabelText("research, 1 agent"));
    await vi.waitFor(() => expect(useStore.getState().railGroup).toBe(RESEARCH));
    expect(useStore.getState().selected).toBe("Chief");

    fireEvent.click(screen.getByLabelText("All groups, 2 agents"));
    fireEvent.click(row("Chief of research"));
    fireEvent.click(screen.getByLabelText("research, 1 agent"));
    await vi.waitFor(() => expect(useStore.getState().railGroup).toBe(RESEARCH));
    expect(useStore.getState().selected).toBe("Chief of research");
  });

  it("keeps Compost in settings even when deleted agents are waiting", () => {
    draw(
      [group("everyone")],
      [agent("Manager"), agent("Scribe", { lifecycle: "terminated", discardedAt: 1_000 })],
    );
    expect(screen.queryByRole("button", { name: /compost/i })).toBeNull();
    expect(screen.getByRole("button", { name: /app settings/i })).toBeTruthy();
    expect(screen.queryByText("Scribe")).toBeNull();
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

describe("an agent that has stopped and said so", () => {
  const stuckOn = (by: string, over: Partial<Escalation> = {}): Escalation => ({
    id: `esc-${by}`,
    agentId: by,
    groupId: DEFAULT_GROUP,
    runId: "run-1",
    summary: "The deploy needs a key only you have.",
    raisedAt: Date.now() - 2 * 24 * 60 * 60 * 1000,
    saidAt: Date.now(),
    times: 3,
    clearedAt: null,
    ...over,
  });

  it("says so on the row, with how long it has been", () => {
    draw([group("everyone")], [agent("Manager")], {}, [stuckOn("Manager")]);
    expect(screen.getByText("stuck 2d")).toBeTruthy();
  });

  it("marks the count for you as waiting only while something is", () => {
    const { unmount } = draw([group("everyone")], [agent("Manager")]);
    const quiet = screen.getByRole("button", { name: /For you/ }).lastElementChild;
    expect(quiet?.textContent).toBe("0");
    expect(quiet?.hasAttribute("data-waiting")).toBe(false);
    unmount();

    draw([group("everyone")], [agent("Manager")], {}, [stuckOn("Manager")]);
    const waiting = screen.getByRole("button", { name: /For you/ }).lastElementChild;
    expect(waiting?.textContent).toBe("1");
    expect(waiting?.getAttribute("data-waiting")).toBe("true");
  });

  it("says which machine a typing agent is on, while a call to it is in flight", () => {
    draw([group("everyone")], [agent("Manager")], { Manager: { state: "thinking" } });
    expect(screen.getByText("typing")).toBeTruthy();

    act(() => {
      useStore.setState({
        trail: {
          Manager: [
            {
              callId: "c1",
              name: "use_screen",
              arguments: { action: "look" },
              done: null,
              startedAt: 0,
            },
          ],
        },
      });
    });
    expect(screen.getByText("on its computer")).toBeTruthy();

    act(() => {
      useStore.setState({
        trail: {
          Manager: [{ callId: "c2", name: "browse", arguments: {}, done: null, startedAt: 0 }],
        },
      });
    });
    expect(screen.getByText("in its browser")).toBeTruthy();
    expect(screen.queryByText("typing")).toBeNull();
  });

  it("says it over anything the agent happens to be doing right now", () => {
    draw([group("everyone")], [agent("Manager")], { Manager: { state: "thinking" } }, [
      stuckOn("Manager"),
    ]);
    expect(screen.getByText("stuck 2d")).toBeTruthy();
    expect(screen.queryByText("typing")).toBeNull();
  });

  it("gives way to a parked turn, which is the more urgent version of itself", () => {
    draw([group("everyone")], [agent("Manager")], { Manager: { state: "awaitingApproval" } }, [
      stuckOn("Manager"),
    ]);
    expect(screen.getByText("needs you")).toBeTruthy();
  });

  it("leaves every other row alone", () => {
    draw([group("everyone")], [agent("Manager"), agent("Reader", { railOrder: 1 })], {}, [
      stuckOn("Manager"),
    ]);
    expect(screen.getAllByText("stuck 2d")).toHaveLength(1);
  });
});

it("lets a touch scroll the rail without rearranging agents", () => {
  const { container } = draw([group("Crew")], [agent("Ada"), agent("Lin")]);
  fireEvent.pointerDown(row("Ada"), {
    button: 0,
    pointerType: "touch",
    clientX: 100,
    clientY: 300,
  });
  fireEvent.pointerMove(window, { pointerType: "touch", clientX: 100, clientY: 200 });
  expect(container.querySelector('.rail[data-dragging="true"]')).toBeNull();
  fireEvent.pointerUp(window, { pointerType: "touch", clientX: 100, clientY: 200 });
  expect(moveAgent).not.toHaveBeenCalled();
});
