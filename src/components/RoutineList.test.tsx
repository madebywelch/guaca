import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Routine } from "../lib/types";
import { RoutineList } from "./RoutineList";

const agentRoutines = vi.fn<() => Promise<Routine[]>>();

vi.mock("../lib/ipc", () => ({
  api: { agentRoutines: () => agentRoutines() },
}));

/** 2025-06-10 at 09:28 local, which is what the labels are written against. */
const MORNING = new Date(2025, 5, 10, 9, 28, 0, 0).getTime();

function routine(over: Partial<Routine> = {}): Routine {
  return {
    id: "r1",
    agentId: "a1",
    name: "Boss commitment nudge",
    what: "check what I promised and remind me",
    trigger: "weekdays",
    active: true,
    nextRunAt: MORNING,
    lastRunAt: null,
    createdAt: 0,
    ...over,
  };
}

describe("RoutineList", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    agentRoutines.mockResolvedValue([]);
  });

  it("reads a routine as a name and a cadence, and nothing else", async () => {
    agentRoutines.mockResolvedValue([routine()]);
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);

    expect(await screen.findByText("Boss commitment nudge")).toBeTruthy();
    expect(screen.getByText(/Weekdays at 9:28/)).toBeTruthy();
    // The instruction is not in the row at all. It runs to several sentences,
    // and a list that drew it was one routine tall.
    expect(screen.queryByText("check what I promised and remind me")).toBeNull();
  });

  it("cuts an unnamed routine down to a title instead of drawing the whole instruction", async () => {
    // Agents set routines for themselves with the `schedule` tool and need not
    // name them, and their instructions are long by design.
    agentRoutines.mockResolvedValue([
      routine({
        name: "",
        what: "Publish on the day only. America/New_York. Manager already cleared this set, so check the feed first.",
      }),
    ]);
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);

    expect(await screen.findByText("Publish on the day only")).toBeTruthy();
    expect(screen.queryByText(/America\/New_York/)).toBeNull();
  });

  it("says which routines are switched off, and when they last ran", async () => {
    // Off is not deleted. It still has to be findable, and it must not claim a
    // next firing it is never going to make. When it last ran is the only news
    // left about it, and the answer to "was this ever working".
    agentRoutines.mockResolvedValue([
      routine({ active: false, lastRunAt: MORNING - 2 * 86_400_000 }),
    ]);
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);

    expect(await screen.findByText(/· off, last ran/)).toBeTruthy();
    expect(screen.queryByText(/next in/)).toBeNull();
  });

  it("says the day a weekly routine keeps, not just the hour", async () => {
    // Nothing else on screen records which day it is: the row said "Every week
    // at 9:28 AM" whichever day that was.
    agentRoutines.mockResolvedValue([routine({ trigger: "weekly" })]);
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);
    expect(await screen.findByText(/Every Tuesday at 9:28/)).toBeTruthy();
  });

  it("draws a routine waiting on an event without counting down to anything", async () => {
    // Nothing delivers an event yet, so this is a row a future build writes and
    // this one has to read. A countdown here would be to a moment nothing is
    // holding.
    agentRoutines.mockResolvedValue([
      routine({ trigger: "event:stripe/invoice.payment_failed", nextRunAt: null }),
    ]);
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);

    expect(await screen.findByText(/When Stripe reports invoice.payment_failed/)).toBeTruthy();
    expect(screen.getByText(/· waiting/)).toBeTruthy();
    expect(screen.queryByText(/next in/)).toBeNull();
  });

  it("opens a routine, and opens an empty one from the plus", async () => {
    const onOpen = vi.fn();
    agentRoutines.mockResolvedValue([routine()]);
    render(<RoutineList agentId="a1" onOpen={onOpen} />);

    fireEvent.click(await screen.findByText("Boss commitment nudge"));
    expect(onOpen).toHaveBeenCalledWith("r1");

    fireEvent.click(screen.getByRole("button", { name: "Add a routine" }));
    expect(onOpen).toHaveBeenCalledWith("new");
  });

  it("says so when there is nothing standing", async () => {
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);
    expect(await screen.findByText(/Nothing standing/)).toBeTruthy();
  });
});
