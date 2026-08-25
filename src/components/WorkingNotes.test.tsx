import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { WorkingNote } from "../lib/types";
import { ago, WorkingNotes } from "./WorkingNotes";

const agentWorkingNotes = vi.fn<(id: string) => Promise<WorkingNote[]>>();
const clearAgentWorkingNotes = vi.fn<(id: string) => Promise<void>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentWorkingNotes: (id: string) => agentWorkingNotes(id),
    clearAgentWorkingNotes: (id: string) => clearAgentWorkingNotes(id),
  },
}));

const HOUR = 3_600_000;
const DAY = 24 * HOUR;

function note(agoMs: number, body: string): WorkingNote {
  return { at: Date.now() - agoMs, body };
}

describe("WorkingNotes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    agentWorkingNotes.mockResolvedValue([]);
    clearAgentWorkingNotes.mockResolvedValue(undefined);
    useStore.setState({ workingNotesVersion: {} });
  });

  it("draws each note beside how long ago it was written", async () => {
    // The age is the whole reason the list is worth reading: it is what tells
    // the operator the difference between an agent working and an agent stuck.
    agentWorkingNotes.mockResolvedValue([
      note(6 * DAY, "asked the paralegal for the regulatory read"),
      note(2 * HOUR, "handed the scope document over"),
    ]);
    render(<WorkingNotes agentId="a1" />);

    expect(await screen.findByText("asked the paralegal for the regulatory read")).toBeTruthy();
    expect(screen.getByText("6d ago")).toBeTruthy();
    expect(screen.getByText("2h ago")).toBeTruthy();
  });

  it("says what the panel is for only where there is nothing in it", async () => {
    render(<WorkingNotes agentId="a1" />);
    expect(await screen.findByText(/Nothing in flight/)).toBeTruthy();

    agentWorkingNotes.mockResolvedValue([note(0, "waiting on Robert")]);
    act(() => {
      useStore.getState().applyEvent({ type: "workingNotesChanged", agentId: "a1" });
    });

    await waitFor(() => expect(screen.queryByText(/Nothing in flight/)).toBeNull());
  });

  it("reads itself again when the agent notes something", async () => {
    // The operator reads this panel while the agent works, which is exactly
    // when it moves. Same argument the memory panel above it has.
    render(<WorkingNotes agentId="a1" />);
    await waitFor(() => expect(agentWorkingNotes).toHaveBeenCalledTimes(1));

    agentWorkingNotes.mockResolvedValue([note(0, "handed the scope document over")]);
    act(() => {
      useStore.getState().applyEvent({ type: "workingNotesChanged", agentId: "a1" });
    });

    expect(await screen.findByText("handed the scope document over")).toBeTruthy();
  });

  it("ignores a note belonging to another agent", async () => {
    render(<WorkingNotes agentId="a1" />);
    await waitFor(() => expect(agentWorkingNotes).toHaveBeenCalledTimes(1));

    act(() => {
      useStore.getState().applyEvent({ type: "workingNotesChanged", agentId: "a2" });
    });

    expect(agentWorkingNotes).toHaveBeenCalledTimes(1);
  });

  it("does not react to the memory beside it moving", async () => {
    // Two counters rather than one. Notes are written far more often than
    // memory, so a shared counter would have every note refetch a page that
    // has not changed.
    render(<WorkingNotes agentId="a1" />);
    await waitFor(() => expect(agentWorkingNotes).toHaveBeenCalledTimes(1));

    act(() => {
      useStore.getState().applyEvent({ type: "memoryChanged", agentId: "a1" });
    });

    expect(agentWorkingNotes).toHaveBeenCalledTimes(1);
  });

  it("offers no way to clear a list that is already empty", async () => {
    render(<WorkingNotes agentId="a1" />);
    await screen.findByText(/Nothing in flight/);
    expect(screen.queryByRole("button", { name: "Clear" })).toBeNull();
  });

  it("clears every note when the operator says the work is done", async () => {
    agentWorkingNotes.mockResolvedValue([note(DAY, "waiting on Robert")]);
    render(<WorkingNotes agentId="a1" />);
    await screen.findByText("waiting on Robert");

    fireEvent.click(screen.getByRole("button", { name: "Clear" }));

    await waitFor(() => expect(clearAgentWorkingNotes).toHaveBeenCalledWith("a1"));
    expect(await screen.findByText(/Nothing in flight/)).toBeTruthy();
  });

  it("draws the read that failed rather than an empty list", async () => {
    // An empty panel is a claim that the agent has nothing in flight, which is
    // a different thing from not having been able to find out.
    agentWorkingNotes.mockRejectedValue(new Error("database is locked"));
    render(<WorkingNotes agentId="a1" />);

    expect(await screen.findByText(/database is locked/)).toBeTruthy();
  });
});

describe("ago", () => {
  const now = 1_000_000_000_000;

  it("says the coarsest unit that is still true", () => {
    expect(ago(now, now)).toBe("just now");
    expect(ago(now - 90_000, now)).toBe("1m ago");
    expect(ago(now - 3 * HOUR, now)).toBe("3h ago");
    expect(ago(now - 50 * HOUR, now)).toBe("2d ago");
  });

  it("reads a note from the future as current rather than negative", () => {
    // Two clocks, or a note written either side of a system time change.
    // "-1d ago" is worse than slightly wrong.
    expect(ago(now + 60_000, now)).toBe("just now");
  });
});
