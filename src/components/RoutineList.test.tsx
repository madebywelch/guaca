import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { Routine } from "../lib/types";
import { RoutineList } from "./RoutineList";

const agentRoutines = vi.fn<() => Promise<Routine[]>>();

vi.mock("../lib/ipc", () => ({
  api: { agentRoutines: () => agentRoutines() },
}));

const MORNING = new Date(2025, 5, 10, 9, 28, 0, 0).getTime();

function routine(over: Partial<Routine> = {}): Routine {
  return {
    id: "r1",
    agentId: "a1",
    name: "Boss commitment nudge",
    what: "check what I promised and remind me",
    trigger: "weekdays",
    active: true,
    skipIfWorking: false,
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
    useStore.setState({ routineVersion: {} });
  });

  it("reads a routine as a name and a cadence, and nothing else", async () => {
    agentRoutines.mockResolvedValue([routine()]);
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);

    expect(await screen.findByText("Boss commitment nudge")).toBeTruthy();
    expect(screen.getByText(/Weekdays at 9:28/)).toBeTruthy();
    expect(screen.queryByText("check what I promised and remind me")).toBeNull();
  });

  it("cuts an unnamed routine down to a title instead of drawing the whole instruction", async () => {
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
    agentRoutines.mockResolvedValue([
      routine({ active: false, lastRunAt: MORNING - 2 * 86_400_000 }),
    ]);
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);

    expect(await screen.findByText(/· off, last ran/)).toBeTruthy();
    expect(screen.queryByText(/next in/)).toBeNull();
  });

  it("says the day a weekly routine keeps, not just the hour", async () => {
    agentRoutines.mockResolvedValue([routine({ trigger: "weekly" })]);
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);
    expect(await screen.findByText(/Every Tuesday at 9:28/)).toBeTruthy();
  });

  it("draws a routine waiting on an event without counting down to anything", async () => {
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

  it("shows the empty state with a failed read and clears the error on refresh", async () => {
    agentRoutines.mockRejectedValueOnce(new Error("Read failed"));
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);

    expect(await screen.findByText("Read failed")).toBeTruthy();
    expect(screen.getByText("Nothing standing.")).toBeTruthy();

    agentRoutines.mockResolvedValueOnce([routine()]);
    act(() => useStore.getState().applyEvent({ type: "routinesChanged", agentId: "a1" }));
    expect(await screen.findByText("Boss commitment nudge")).toBeTruthy();
    expect(screen.queryByText("Read failed")).toBeNull();
  });

  it("keeps the previous rows while a refresh is in flight", async () => {
    agentRoutines.mockResolvedValueOnce([routine()]);
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);
    expect(await screen.findByText("Boss commitment nudge")).toBeTruthy();

    let finish!: (routines: Routine[]) => void;
    agentRoutines.mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    act(() => useStore.getState().applyEvent({ type: "routinesChanged", agentId: "a1" }));
    expect(screen.getByText("Boss commitment nudge")).toBeTruthy();
    await act(async () => finish([]));
    expect(screen.getByText("Nothing standing.")).toBeTruthy();
  });

  it("reads itself again when the agent it belongs to changes its own schedule", async () => {
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);
    expect(await screen.findByText(/Nothing standing/)).toBeTruthy();

    agentRoutines.mockResolvedValue([routine()]);
    act(() => {
      useStore.getState().applyEvent({ type: "routinesChanged", agentId: "a1" });
    });

    expect(await screen.findByText("Boss commitment nudge")).toBeTruthy();
  });

  it("ignores a schedule change belonging to another agent", async () => {
    render(<RoutineList agentId="a1" onOpen={vi.fn()} />);
    expect(await screen.findByText(/Nothing standing/)).toBeTruthy();
    expect(agentRoutines).toHaveBeenCalledTimes(1);

    act(() => {
      useStore.getState().applyEvent({ type: "routinesChanged", agentId: "a2" });
    });

    expect(agentRoutines).toHaveBeenCalledTimes(1);
  });
});
