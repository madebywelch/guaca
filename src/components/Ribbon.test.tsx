import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { type Ribbon as Layout, layoutRibbon } from "../lib/ribbon";
import { useStore } from "../lib/store";
import { anAgent, anApproval, anOutcome } from "../test-fixtures";
import { Ribbon } from "./Ribbon";

const at = (day: number, hour: number, minute = 0) =>
  new Date(2026, 10, day, hour, minute).getTime();

it("runs in the one zone its labels are written for", () => {
  expect(Intl.DateTimeFormat().resolvedOptions().timeZone).toBe("America/New_York");
  expect(new Date(at(16, 9, 12)).toString()).toContain("Eastern Standard Time");
});

const AGENTS = ["dara", "marlow", "ines", "kit", "wren", "pip"].map((id) => anAgent(id));

function morning(): Layout {
  return layoutRibbon({
    now: at(16, 9, 12),
    agents: AGENTS,
    activity: { marlow: { state: "thinking" } },
    outcomes: [
      anOutcome({ agentId: "kit", finishedAt: at(15, 23), seenAt: at(15, 23, 40) }),
      anOutcome({ agentId: "ines", startedAt: at(16, 2, 40), finishedAt: at(16, 2, 56) }),
    ],
    approvals: [anApproval({ agentId: "dara", createdAt: at(16, 9, 5), lapsesAt: at(16, 9, 15) })],
    upcoming: [
      {
        routineId: "r1",
        agentId: "ines",
        groupId: "g",
        name: "Changelog type check",
        nextRunAt: at(17, 9),
      },
    ],
    occasions: [],
    faceSpan: 0.05,
  });
}

const openAgent = vi.fn<(id: string) => void>();
const openPile = vi.fn<() => void>();

function draw(ribbon = morning()) {
  render(<Ribbon ribbon={ribbon} onOpenAgent={openAgent} onOpenPile={openPile} />);
  return screen.getByRole("list", { name: "Today's timeline" });
}

beforeEach(() => {
  openAgent.mockReset();
  openPile.mockReset();
  useStore.setState({ agents: AGENTS, activity: { marlow: { state: "thinking" } } });
});

describe("what a screen reader hears", () => {
  it("is one list, every face, the pile and the schedule in it", () => {
    const list = draw();
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(6);
  });

  it("names each face with who, its state and its time", () => {
    const list = draw();
    expect(
      within(list).getByRole("button", { name: "Dara, needs you, asked 9:05 AM" }),
    ).toBeTruthy();
    expect(within(list).getByRole("button", { name: "Marlow, working now" })).toBeTruthy();
    expect(within(list).getByRole("button", { name: "Ines, finished 2:56 AM" })).toBeTruthy();
  });

  it("says how many are asleep on the pile", () => {
    const list = draw();
    expect(within(list).getByRole("button", { name: "3 asleep" })).toBeTruthy();
  });

  it("calls the pile how many more, not asleep, once it holds faces moved there for room", () => {
    const list = draw({ ...morning(), pile: ["kit", "wren", "pip", "ines"], crowded: 1 });
    expect(within(list).getByRole("button", { name: "4 more" })).toBeTruthy();
    expect(within(list).queryByRole("button", { name: /asleep/ })).toBeNull();
  });

  it("reads the strip in the order it is drawn, left to right", () => {
    const ribbon = morning();
    const list = draw(ribbon);
    const items = within(list).getAllByRole("listitem");
    expect(items[0]?.className).toBe("ribbon__pile");
    const xs = items.slice(1).map((item) => Number(item.style.getPropertyValue("--ribbon-x")));
    expect(xs).toEqual([...xs].sort((a, b) => a - b));
    expect(new Set(xs).size).toBe(xs.length);
  });

  it("draws faces too many to fit as one count that opens the Groups sheet", () => {
    const crowd = ["kit", "wren", "pip", "ines", "dara", "marlow"];
    useStore.setState({
      agents: AGENTS,
      activity: Object.fromEntries(crowd.map((id) => [id, { state: "thinking" as const }])),
    });
    const ribbon = layoutRibbon({
      now: at(16, 9, 12),
      agents: AGENTS,
      activity: Object.fromEntries(crowd.map((id) => [id, { state: "thinking" as const }])),
      outcomes: [],
      approvals: [],
      upcoming: [],
      occasions: [],
      faceSpan: 52 / 375,
      charSpan: 8 / 375,
    });
    const list = draw(ribbon);
    const count = within(list).getByRole("button", { name: /^\+\d+ working: / });
    expect(count.textContent).toMatch(/^\+\d+ working$/);
    fireEvent.click(count);
    expect(openPile).toHaveBeenCalledTimes(1);
  });

  it("reads a scheduled run with its day, and does not offer it as a button", () => {
    const list = draw();
    const run = within(list).getByText("Changelog type check, Tue 9:00 AM");
    expect(run.closest("li")).toBeTruthy();
    expect(run.closest("button")).toBeNull();
  });

  it("reads where a request stops waiting", () => {
    const list = draw();
    expect(within(list).getByText("Dara stops waiting, 9:15 AM")).toBeTruthy();
  });

  it("keeps the alert out of the face's drawing", () => {
    const list = draw();
    const dara = within(list).getByRole("button", { name: /^Dara/ });
    const alert = dara.querySelector(".ribbon__alert");
    expect(alert?.textContent).toBe("!");
    expect(alert?.closest(".avatar")).toBeNull();
  });

  it("draws no pile when nobody is asleep", () => {
    const ribbon = { ...morning(), pile: [] };
    const list = draw(ribbon);
    expect(within(list).queryByRole("button", { name: /asleep/ })).toBeNull();
  });
});

describe("the keyboard", () => {
  const names = () =>
    screen
      .getAllByRole("button")
      .filter((button) => button.tabIndex === 0)
      .map((button) => button.textContent);

  it("is one stop on the Tab key for the whole strip", () => {
    draw();
    expect(names()).toHaveLength(1);
  });

  it("walks the pile and the faces in time order with the arrows", () => {
    const list = draw();
    const pile = within(list).getByRole("button", { name: "3 asleep" });
    act(() => pile.focus());
    fireEvent.keyDown(pile, { key: "ArrowRight" });
    expect(document.activeElement).toBe(within(list).getByRole("button", { name: /^Ines/ }));
    fireEvent.keyDown(document.activeElement as Element, { key: "ArrowRight" });
    expect(document.activeElement).toBe(within(list).getByRole("button", { name: /^Dara/ }));
    fireEvent.keyDown(document.activeElement as Element, { key: "ArrowLeft" });
    expect(document.activeElement).toBe(within(list).getByRole("button", { name: /^Ines/ }));
    expect((document.activeElement as HTMLElement).tabIndex).toBe(0);
    expect(names()).toHaveLength(1);
  });

  it("goes to either end with Home and End, and stops there", () => {
    const list = draw();
    const pile = within(list).getByRole("button", { name: "3 asleep" });
    act(() => pile.focus());
    fireEvent.keyDown(pile, { key: "End" });
    const last = within(list).getByRole("button", { name: /^Marlow/ });
    expect(document.activeElement).toBe(last);
    fireEvent.keyDown(last, { key: "ArrowRight" });
    expect(document.activeElement).toBe(last);
    fireEvent.keyDown(last, { key: "Home" });
    expect(document.activeElement).toBe(pile);
    fireEvent.keyDown(pile, { key: "ArrowLeft" });
    expect(document.activeElement).toBe(pile);
  });
});

describe("a click", () => {
  it("on a face opens that agent", () => {
    const list = draw();
    fireEvent.click(within(list).getByRole("button", { name: /^Dara/ }));
    expect(openAgent).toHaveBeenCalledWith("dara");
    expect(openPile).not.toHaveBeenCalled();
  });

  it("on the pile opens the pile", () => {
    const list = draw();
    fireEvent.click(within(list).getByRole("button", { name: "3 asleep" }));
    expect(openPile).toHaveBeenCalledTimes(1);
    expect(openAgent).not.toHaveBeenCalled();
  });
});

describe("an agent's page", () => {
  it("marks whose page is open and steps the rest back", () => {
    render(
      <Ribbon
        ribbon={morning()}
        onOpenAgent={openAgent}
        onOpenPile={openPile}
        openAgentId="marlow"
      />,
    );
    const strip = screen.getByRole("navigation", { name: "Agents" });
    expect(strip.getAttribute("data-open")).toBe("some");
    const list = within(strip).getByRole("list", { name: "Today's timeline" });
    expect(
      within(list)
        .getByRole("button", { name: /^Marlow/ })
        .getAttribute("aria-current"),
    ).toBe("page");
    expect(
      within(list).getByRole("button", { name: /^Dara/ }).getAttribute("aria-current"),
    ).toBeNull();
  });

  it("dims nobody when the open agent is asleep in the pile", () => {
    render(
      <Ribbon ribbon={morning()} onOpenAgent={openAgent} onOpenPile={openPile} openAgentId="kit" />,
    );
    expect(screen.getByRole("navigation", { name: "Agents" }).getAttribute("data-open")).toBeNull();
  });
});

describe("a coming item", () => {
  it("shows its time even with no room to say what it is", () => {
    const ribbon = morning();
    const crowded = {
      ...ribbon,
      scheduled: ribbon.scheduled.map((item) => ({ ...item, words: false })),
    };
    const { container } = render(
      <Ribbon ribbon={crowded} onOpenAgent={openAgent} onOpenPile={openPile} />,
    );
    const item = container.querySelector(".ribbon__item");
    expect(item?.querySelector(".ribbon__item-when")?.textContent).toBe("9:00 AM");
    expect(item?.querySelector(".ribbon__item-what")).toBeNull();
  });
});
