import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { Activity, AgentCard, Escalation, Group } from "../lib/types";
import { aGroup, DEFAULT_GROUP } from "../test-fixtures";
import { Sidebar } from "./Sidebar";

const moveAgent =
  vi.fn<(id: string, groupId: string, before: string | null) => Promise<AgentCard>>();
const setAgentPinned = vi.fn<(id: string, pinned: boolean) => Promise<AgentCard>>();

vi.mock("../lib/ipc", () => ({
  api: {
    listAgents: async () => [],
    listGroups: async () => [],
    listRepositories: async () => [],
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
    browserConsent: "open",
    repositoryId: null,
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
  return {
    ...render(
      <Sidebar
        onEditAgent={vi.fn()}
        onEditGroup={vi.fn()}
        onOpenCafeteria={vi.fn()}
        onOpenCompost={vi.fn()}
        onOpenSettings={vi.fn()}
        onOpenSearch={vi.fn()}
        onNewAgent={onNewAgent}
        onNewGroup={onNewGroup}
        onOpenMenu={vi.fn()}
      />,
    ),
    onNewAgent,
    onNewGroup,
  };
}

/** A row, by the name written in it. */
function row(name: string): HTMLElement {
  const found = screen
    .getAllByRole("button")
    .find((node) => node.className === "agent-row" && node.textContent?.startsWith(name));
  if (!found) throw new Error(`no row for ${name}`);
  return found;
}

/** Every row the rail is drawing, top to bottom. */
function names(): (string | null)[] {
  return [...document.querySelectorAll(".agent-row__name")].map((n) => n.textContent);
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

describe("the plus", () => {
  /**
   * Two things the channel header it used to sit in could not give it. There it
   * was drawn at the far right of the reading column beside the agent's own
   * actions menu, which is not where anybody looks to add an agent; and on an
   * empty workspace there is no channel open to draw a header at all.
   */
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

  it("pins a row dropped on a pinned peer, and lands it among the pins", async () => {
    // The row landed on says both things at once: which crew, and whether the
    // place aimed at is the band a pin holds or the rest of the crew below it.
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

    // In front of Scribe, not at the end: an agent arriving from another
    // section has no place in this one to have traveled from, so there is no
    // direction to read and it takes the place of what it was dropped on.
    expect(setAgentPinned).toHaveBeenCalledWith("Manager", false);
    expect(moveAgent).toHaveBeenCalledWith("Manager", DEFAULT_GROUP, "Scribe");
  });
});

describe("pins", () => {
  const RESEARCH = "00000000-0000-4000-8000-000000000002";

  /** The crew, and one other so the strip is drawn and can be clicked. */
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
    // The one that was reported: pinning a row while looking inside a crew did
    // nothing until the operator went back out to the overview, because the
    // section a pin moved a row into was only drawn out there.
    draw(crews(), roster());

    expect(names()).toEqual(["Chef", "Manager", "Cook", "Reader"]);
    expect(screen.queryByText("Pinned")).toBeNull();

    fireEvent.click(screen.getByLabelText("everyone, 3 agents"));
    expect(names()).toEqual(["Chef", "Manager", "Cook"]);
  });

  it("marks the pinned row, because being first is not a state", () => {
    // A crew whose pin is also the row the operator arranged at the top looks
    // exactly like a pin that did nothing, and the mark is the only thing on
    // screen that tells those two apart.
    draw(crews(), roster());

    expect(row("Chef").querySelector(".agent-row__pin")).toBeTruthy();
    expect(row("Manager").querySelector(".agent-row__pin")).toBeNull();
  });

  it("keeps the pin when the agent is moved to another crew", async () => {
    // A pin is a standing instruction about one agent. Changing who it works
    // with is not a decision to drop it, and the drop says nothing about the
    // band either way.
    draw(crews(), roster());

    await dragTo(row("Chef"), screen.getByLabelText("research, 1 agent"));

    expect(moveAgent).toHaveBeenCalledWith("Chef", RESEARCH, null);
    expect(setAgentPinned).not.toHaveBeenCalled();
  });
});

describe("what the rail offers", () => {
  it("does not offer the flow board", () => {
    // It sat here, under the wordmark and above the search box: the first thing
    // in the app after Guaca's own name, and one of the least pressed things in
    // it. Who spoke to whom and what a run cost is analysis, so it is in a
    // crew's settings where somebody who has decided to look into something
    // will find it.
    draw([group("everyone")], [agent("Manager")]);
    expect(screen.queryByRole("button", { name: /activity/i })).toBeNull();
  });
});

describe("groups as places", () => {
  const RESEARCH = "00000000-0000-4000-8000-000000000002";

  /**
   * Gives the two boxes the proximity decision reads a size.
   *
   * jsdom does no layout, so both come back as zeros and every threshold
   * collapses onto the same pixel. These are the numbers a real window has: a
   * zone reaching 20px in and starting under the window's own buttons, and a
   * column that is 8px of itself when it is in and 64px when it is out.
   */
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

  /** Whether the crews are drawn out over the rail. */
  function out(): boolean {
    return screen.getByLabelText("Groups").getAttribute("data-out") === "true";
  }

  it("keeps the crews in until the pointer comes at them", () => {
    // The column stood open and charged every window four rem of its width for
    // a choice most operators make a few times a day, in front of the rail that
    // is read constantly.
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
    // macOS floats close, minimize and zoom over the top left corner, which is
    // this column. Coming out on the way to them would put the crews under the
    // pointer at the moment it was aimed at the button that closes the app.
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );
    laid(false);

    fireEvent.pointerMove(window, { clientX: 6, clientY: 18 });
    expect(out()).toBe(false);
  });

  it("stays in for a drag that never goes near it", () => {
    // It came out for the whole of every drag, and the column comes out over
    // the rail: picking up a row anywhere in the window covered the left edge
    // of every row behind it, which is what a reorder is aimed at.
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
    // A drop onto a circle is the one thing this column is load-bearing for. A
    // column that slid away as the hand carried a row back across the app would
    // take the target with it half way through the gesture.
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

  it("draws one circle per group, and the faces in it", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );

    expect(screen.getByLabelText("Groups")).toBeTruthy();
    expect(screen.getByLabelText("research, 1 agent")).toBeTruthy();
    expect(screen.getByTitle("Reader")).toBeTruthy();
  });

  // The circle is how a crew is told apart at 38px, and every crew of two or
  // more used to draw the same square of four. Where the faces stand is now the
  // crew's size, so the ring holds more than four and says so when it cannot.
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
    // The column is the only thing on screen about the crews the operator is
    // not looking at, so it has to carry the one state that is waiting on a
    // person. Said in the label as well as drawn, because the mark is a number
    // in a corner and nothing reading the page aloud can see it.
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
      { Reader: { state: "awaitingApproval" } },
    );

    const orb = screen.getByLabelText("research, 1 agent, 1 turn waiting on you");
    expect(orb.querySelector(".orb__waiting")?.textContent).toBe("1");
  });

  // A dot says a crew needs you. A number says how much of your time it needs,
  // which is what an operator triaging a dozen crews is choosing between.
  it("counts the parked turns rather than reporting that there are some", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [
        agent("Manager"),
        agent("Reader", { groupId: RESEARCH, railOrder: 1 }),
        agent("Writer", { groupId: RESEARCH, railOrder: 2 }),
      ],
      { Reader: { state: "awaitingApproval" }, Writer: { state: "awaitingApproval" } },
    );

    const orb = screen.getByLabelText("research, 2 agents, 2 turns waiting on you");
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

  // Faces do not identify a crew. The cafeteria is a copy machine with a fixed
  // avatar and color per preset, so two crews hired from the same counters draw
  // the same faces, and above six every crew draws six and a count. Reading a
  // name meant clicking, which is the navigation the column replaced.
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

  // A column reachable only with a pointer is a column a keyboard operator has
  // to click through, which is the complaint this answers for everyone else.
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

  // The full name, not the one the heading has room for. A tag that ellipsed
  // would move where the name is cut off rather than stop cutting it.
  it("draws the whole name, however long it is", () => {
    const long = "Customer research and competitive intelligence, EMEA";
    draw([group("everyone"), group(long, null, RESEARCH)], [agent("Manager")]);

    const orb = screen.getByLabelText(`${long}, 0 agents`);
    fireEvent.pointerEnter(orb);
    expect(orb.querySelector(".orb__tag-name")?.textContent).toBe(long);
  });

  // The circle carries the ring and the corner count on two channels. The tag
  // is where they are put into words, so triage does not need the crew opened.
  it("says what the crew is doing under its name, in the label's own words", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
      { Reader: { state: "awaitingApproval" } },
    );

    const orb = screen.getByLabelText("research, 1 agent, 1 turn waiting on you");
    fireEvent.pointerEnter(orb);
    expect(orb.querySelector(".orb__tag-note")?.textContent).toBe("1 agent, 1 turn waiting on you");
  });

  // Said once, as the button's own label. Drawn into the tree a second time it
  // is the crew announced twice, the second time as text nobody can reach.
  it("keeps the tag out of the accessibility tree", () => {
    draw(
      [group("everyone"), group("research", null, RESEARCH)],
      [agent("Manager"), agent("Reader", { groupId: RESEARCH, railOrder: 1 })],
    );

    const orb = screen.getByLabelText("research, 1 agent");
    fireEvent.pointerEnter(orb);
    expect(orb.querySelector(".orb__tag")?.getAttribute("aria-hidden")).toBe("true");
  });

  // A bare number in a circle is as opaque as an unnamed crew, and it is the
  // first thing in the column.
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

  // Aiming a drag at a crew is when the circle most has to say which crew it
  // is, and it is the one moment the native tooltip is suppressed.
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

  // The circle used to be a toggle, so the gesture for going into a crew went
  // in and straight back out again, and a click on the crew you were already
  // in put you in the overview. There is a circle for the overview at the top
  // of the column, on screen for the whole of both gestures.
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

  // Which circle is lit is the whole argument for the column: "which crew am I
  // in" is meant to have an answer in a fixed place. What that mark is drawn in
  // is `styles.test.ts`, which is where it was invisible for a year.
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

  // The heading ellipses whatever does not fit the rail, and after clicking a
  // circle it is the only place the name is drawn at all.
  it("keeps the whole name on the heading the rail cuts short", () => {
    const long = "Customer research and competitive intelligence, EMEA";
    draw([group("everyone"), group(long, null, RESEARCH)], [agent("Manager")]);

    expect(document.querySelector(".rail__group-name")?.getAttribute("title")).toBe("everyone");

    fireEvent.click(screen.getByLabelText(`${long}, 0 agents`));
    expect(document.querySelector(".rail__open-name")?.getAttribute("title")).toBe(long);
  });

  // The name is the heading of the whole column once the rail is inside a crew,
  // and it was the only thing on that line able to give up width. Both readouts
  // that used to take it are gone: the count entirely, and the spend into the
  // card that hovering the heading opens.
  it("keeps the crew's spend and its count off the line the name is on", () => {
    draw([group("everyone"), group("StopTheScam", null, RESEARCH)], [agent("Manager")]);
    fireEvent.click(screen.getByLabelText("StopTheScam, 0 agents"));

    const head = document.querySelector(".rail__open-head");
    expect(head?.textContent).toBe("StopTheScam⚙");
    expect(document.querySelector(".spend")).toBeNull();
  });

  // A crew's heading is a band across the top of its own rows, so a pointer on
  // its way to an agent crosses one every time. Opening on contact flashed a
  // panel over the row being aimed at.
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

  // The pointer is already carrying something, and what the card would cover is
  // the rows being aimed at.
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
    await vi.waitFor(() => expect(useStore.getState().selected).toBeNull());

    fireEvent.click(screen.getByLabelText("All groups, 2 agents"));
    fireEvent.click(row("Chief of research"));
    fireEvent.click(screen.getByLabelText("research, 1 agent"));
    await vi.waitFor(() => expect(useStore.getState().railGroup).toBe(RESEARCH));
    expect(useStore.getState().selected).toBe("Chief of research");
  });

  it("keeps the compost off the footer while it is empty", () => {
    // What keeps the footer two rows for an operator who has never deleted
    // anybody. Its counterpart below is how the compost is found by one who
    // just has: the row turns up at the moment it has a reason to.
    draw([group("everyone")], [agent("Manager")]);
    expect(screen.queryByRole("button", { name: /compost/i })).toBeNull();
  });

  it("offers the compost, and its count, once somebody is in it", () => {
    draw(
      [group("everyone")],
      [agent("Manager"), agent("Scribe", { lifecycle: "terminated", discardedAt: 1_000 })],
    );

    expect(screen.getByRole("button", { name: /compost/i }).textContent).toContain("1");
    // And the agent is not in the rail itself, which is the whole point of
    // having deleted it.
    expect(screen.queryByText("Scribe")).toBeNull();
  });

  it("leaves an agent whose wait is already over out of the count", () => {
    // Terminated with no stamp: deleted before the compost existed, or swept
    // out of it. Counting one would offer a restore that cannot be kept.
    draw([group("everyone")], [agent("Manager"), agent("Ghost", { lifecycle: "terminated" })]);
    expect(screen.queryByRole("button", { name: /compost/i })).toBeNull();
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

  // This column is what somebody scans when they have noticed that nothing is
  // moving. An agent stuck since Tuesday reading as idle is the whole reason
  // the escalation exists, one surface up.
  it("says so on the row, with how long it has been", () => {
    draw([group("everyone")], [agent("Manager")], {}, [stuckOn("Manager")]);
    expect(screen.getByText("stuck 2d")).toBeTruthy();
  });

  // A state that resolves itself must not hide one that does not.
  it("says it over anything the agent happens to be doing right now", () => {
    draw([group("everyone")], [agent("Manager")], { Manager: { state: "thinking" } }, [
      stuckOn("Manager"),
    ]);
    expect(screen.getByText("stuck 2d")).toBeTruthy();
    expect(screen.queryByText("typing")).toBeNull();
  });

  // The one state that outranks it, and only because it is the same statement
  // with a turn parked behind it: that one has ten minutes and this one does not.
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
