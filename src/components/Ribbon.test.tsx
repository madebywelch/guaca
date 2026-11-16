import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { type Ribbon as Layout, layoutRibbon } from "../lib/ribbon";
import { useStore } from "../lib/store";
import { anAgent, anApproval, anOutcome } from "../test-fixtures";
import { Ribbon } from "./Ribbon";

const at = (day: number, hour: number, minute = 0) =>
  new Date(2026, 10, day, hour, minute).getTime();

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
    // The pile, three faces, one run and one lapse point.
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
