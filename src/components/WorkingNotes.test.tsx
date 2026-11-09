import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { WorkingNote } from "../lib/types";
import { ago, notesTransition, WorkingNotes } from "./WorkingNotes";

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

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

describe("WorkingNotes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    localStorage.removeItem("guac.workingNotes");
    agentWorkingNotes.mockResolvedValue([]);
    clearAgentWorkingNotes.mockResolvedValue(undefined);
    useStore.setState({ workingNotesVersion: {} });
  });

  it("starts collapsed and remembers an expanded section across agents", async () => {
    agentWorkingNotes.mockResolvedValue([note(HOUR, "waiting on Robert")]);
    const first = render(<WorkingNotes agentId="a1" />);
    const disclosure = await screen.findByRole("button", {
      // The count and the age are their own spans, and an accessible name
      // joins inline children without the spaces between them.
      name: /Working notes.*1.*newest 1h ago/,
    });
    expect(disclosure.getAttribute("aria-expanded")).toBe("false");
    expect(screen.queryByText("waiting on Robert")).toBeNull();
    fireEvent.click(disclosure);
    expect(disclosure.getAttribute("aria-expanded")).toBe("true");
    first.unmount();
    render(<WorkingNotes agentId="a2" />);
    expect(
      screen.getByRole("button", { name: /Working notes/ }).getAttribute("aria-expanded"),
    ).toBe("true");
  });

  it("draws each note beside how long ago it was written", async () => {
    // The age is the whole reason the list is worth reading: it is what tells
    // the operator the difference between an agent working and an agent stuck.
    agentWorkingNotes.mockResolvedValue([
      note(6 * DAY, "asked the paralegal for the regulatory read"),
      note(2 * HOUR, "handed the scope document over"),
    ]);
    render(<WorkingNotes agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));

    expect(await screen.findByText("asked the paralegal for the regulatory read")).toBeTruthy();
    expect(screen.getByText("6d ago")).toBeTruthy();
    expect(screen.getByText("2h ago")).toBeTruthy();
  });

  it("says what the panel is for only where there is nothing in it", async () => {
    render(<WorkingNotes agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));
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
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));
    await waitFor(() => expect(agentWorkingNotes).toHaveBeenCalledTimes(1));

    agentWorkingNotes.mockResolvedValue([note(0, "handed the scope document over")]);
    act(() => {
      useStore.getState().applyEvent({ type: "workingNotesChanged", agentId: "a1" });
    });

    expect(await screen.findByText("handed the scope document over")).toBeTruthy();
  });

  it("ignores a note belonging to another agent", async () => {
    render(<WorkingNotes agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));
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
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));
    await waitFor(() => expect(agentWorkingNotes).toHaveBeenCalledTimes(1));

    act(() => {
      useStore.getState().applyEvent({ type: "memoryChanged", agentId: "a1" });
    });

    expect(agentWorkingNotes).toHaveBeenCalledTimes(1);
  });

  it("draws the newest few and keeps the older ones behind a button", async () => {
    // The panel shares one column with the agent's screen, its schedule and
    // its memory. A full list of sixteen notes, each wrapping to three lines in
    // a sidebar this narrow, is a screen of text everything else sits below.
    // Which end is kept is the decision: the notes read oldest first, so the
    // tail is where the work is now.
    agentWorkingNotes.mockResolvedValue([
      note(6 * DAY, "asked the paralegal for the read"),
      note(5 * DAY, "chased the vendor"),
      note(4 * DAY, "handed the scope over"),
      note(3 * DAY, "started the summary"),
      note(2 * DAY, "sent the summary to Robert"),
      note(HOUR, "waiting on Robert"),
    ]);
    render(<WorkingNotes agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));

    expect(await screen.findByText("waiting on Robert")).toBeTruthy();
    expect(screen.getByText("handed the scope over")).toBeTruthy();
    expect(screen.queryByText("asked the paralegal for the read")).toBeNull();

    // The count is on the button because the alternative is an operator who
    // cannot tell a list of six from a list of sixteen without opening it.
    fireEvent.click(screen.getByRole("button", { name: "Show 2 older" }));

    expect(screen.getByText("asked the paralegal for the read")).toBeTruthy();
    expect(screen.getByText("chased the vendor")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Show fewer" }));
    expect(screen.queryByText("asked the paralegal for the read")).toBeNull();
  });

  it("offers nothing to open on a list that already fits", async () => {
    agentWorkingNotes.mockResolvedValue([note(HOUR, "waiting on Robert")]);
    render(<WorkingNotes agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));
    await screen.findByText("waiting on Robert");

    expect(screen.queryByRole("button", { name: /older/ })).toBeNull();
  });

  it("offers no way to clear a list that is already empty", async () => {
    render(<WorkingNotes agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));
    await screen.findByText(/Nothing in flight/);
    expect(screen.queryByRole("button", { name: "Clear" })).toBeNull();
  });

  it("clears every note when the operator says the work is done", async () => {
    agentWorkingNotes.mockResolvedValue([note(DAY, "waiting on Robert")]);
    render(<WorkingNotes agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));
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
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));

    expect(await screen.findByText(/database is locked/)).toBeTruthy();
    expect(screen.queryByText(/Nothing in flight/)).toBeNull();
    expect(screen.getByRole("button", { name: "Working notes" })).toBeTruthy();
  });

  it("ignores a snapshot taken before a successful clear", async () => {
    const old = deferred<WorkingNote[]>();
    agentWorkingNotes.mockResolvedValueOnce([note(0, "held")]).mockReturnValueOnce(old.promise);
    render(<WorkingNotes agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));
    await screen.findByText("held");
    act(() => useStore.getState().applyEvent({ type: "workingNotesChanged", agentId: "a1" }));
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    await screen.findByText(/Nothing in flight/);
    await act(async () => old.resolve([note(0, "obsolete")]));
    expect(screen.queryByText("obsolete")).toBeNull();
  });

  it("keeps the latest overlapping read", async () => {
    const old = deferred<WorkingNote[]>();
    const recent = deferred<WorkingNote[]>();
    agentWorkingNotes.mockReturnValueOnce(old.promise).mockReturnValueOnce(recent.promise);
    render(<WorkingNotes agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));
    act(() => useStore.getState().applyEvent({ type: "workingNotesChanged", agentId: "a1" }));
    await act(async () => recent.resolve([note(0, "new")]));
    await act(async () => old.resolve([note(0, "old")]));
    expect(screen.getByText("new")).toBeTruthy();
    expect(screen.queryByText("old")).toBeNull();
  });

  it("retains notes if clear fails", async () => {
    agentWorkingNotes.mockResolvedValueOnce([note(0, "held")]);
    clearAgentWorkingNotes.mockRejectedValueOnce(new Error("clear failed"));
    render(<WorkingNotes agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));
    await screen.findByText("held");
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    await screen.findByText("clear failed");
    expect(screen.getByText("held")).toBeTruthy();
  });

  it("ignores a late read from the previous agent", async () => {
    const old = deferred<WorkingNote[]>();
    agentWorkingNotes.mockReturnValueOnce(old.promise).mockResolvedValueOnce([note(0, "second")]);
    const view = render(<WorkingNotes agentId="a1" />);
    fireEvent.click(screen.getByRole("button", { name: /Working notes/ }));
    view.rerender(<WorkingNotes agentId="a2" />);
    await screen.findByText("second");
    await act(async () => old.resolve([note(0, "first")]));
    expect(screen.getByText("second")).toBeTruthy();
    expect(screen.queryByText("first")).toBeNull();
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

describe("notesTransition", () => {
  it("covers reads, clear, failures, stale responses and agent changes", () => {
    const initial = {
      agentId: "a1",
      generation: 0,
      read: { kind: "loading" as const },
      busy: false,
    };
    const loading = notesTransition(initial, { type: "load", agentId: "a1", generation: 1 });
    const failure = notesTransition(loading, {
      type: "readFailed",
      agentId: "a1",
      generation: 1,
      error: "read",
    });
    expect(failure.read).toEqual({ kind: "failed", error: "read" });
    const ready = notesTransition(loading, {
      type: "loaded",
      agentId: "a1",
      generation: 1,
      notes: [note(0, "held")],
    });
    expect(ready.read.kind).toBe("ready");
    expect(
      notesTransition(ready, { type: "loaded", agentId: "a1", generation: 0, notes: [] }),
    ).toBe(ready);
    expect(
      notesTransition(ready, { type: "readFailed", agentId: "a1", generation: 0, error: "old" }),
    ).toBe(ready);
    const failedRefresh = notesTransition(ready, {
      type: "readFailed",
      agentId: "a1",
      generation: 1,
      error: "read",
    });
    expect(failedRefresh.read).toMatchObject({ kind: "ready", error: "read" });
    const clearing = notesTransition(failedRefresh, { type: "clear", agentId: "a1" });
    expect(clearing).toMatchObject({ busy: true, read: { error: null } });
    const clearFailed = notesTransition(clearing, {
      type: "clearFailed",
      agentId: "a1",
      error: "clear",
    });
    expect(clearFailed).toMatchObject({ busy: false, read: { kind: "ready", error: "clear" } });
    expect(
      notesTransition(initial, { type: "clearFailed", agentId: "a1", error: "clear" }).read,
    ).toEqual({ kind: "failed", error: "clear" });
    const cleared = notesTransition(clearing, { type: "cleared", agentId: "a1", generation: 2 });
    expect(cleared).toMatchObject({
      generation: 2,
      busy: false,
      read: { kind: "ready", notes: [], error: null },
    });
    const switched = notesTransition(cleared, { type: "load", agentId: "a2", generation: 3 });
    expect(switched).toEqual({
      agentId: "a2",
      generation: 3,
      read: { kind: "loading" },
      busy: false,
    });
    expect(notesTransition(switched, { type: "cleared", agentId: "a1", generation: 4 })).toBe(
      switched,
    );
  });
});
