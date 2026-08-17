import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Routine, RoutineDraft, RoutineRun } from "../lib/types";
import { RoutineDetail } from "./RoutineDetail";

const agentRoutines = vi.fn<() => Promise<Routine[]>>();
const routineRuns = vi.fn<() => Promise<RoutineRun[]>>();
const createRoutine = vi.fn<(agentId: string, draft: RoutineDraft) => Promise<Routine>>();
const updateRoutine = vi.fn<(id: string, draft: RoutineDraft) => Promise<Routine>>();
const setRoutineActive = vi.fn<(id: string, active: boolean) => Promise<Routine>>();
const testRoutine = vi.fn<(id: string) => Promise<string>>();
const deleteRoutine = vi.fn<(id: string) => Promise<void>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentRoutines: () => agentRoutines(),
    routineRuns: () => routineRuns(),
    createRoutine: (agentId: string, draft: RoutineDraft) => createRoutine(agentId, draft),
    updateRoutine: (id: string, draft: RoutineDraft) => updateRoutine(id, draft),
    setRoutineActive: (id: string, active: boolean) => setRoutineActive(id, active),
    testRoutine: (id: string) => testRoutine(id),
    deleteRoutine: (id: string) => deleteRoutine(id),
  },
}));

/** 2025-06-10 at 09:28 local. */
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

function open(over: Partial<Routine> = {}) {
  agentRoutines.mockResolvedValue([routine(over)]);
  const onBack = vi.fn();
  render(<RoutineDetail agentId="a1" routineId="r1" onBack={onBack} />);
  return onBack;
}

describe("RoutineDetail", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    agentRoutines.mockResolvedValue([routine()]);
    routineRuns.mockResolvedValue([]);
    createRoutine.mockResolvedValue(routine());
    updateRoutine.mockResolvedValue(routine());
    setRoutineActive.mockImplementation(async (_id, active) => routine({ active }));
    testRoutine.mockResolvedValue("run-1");
    deleteRoutine.mockResolvedValue(undefined);
  });

  it("shows the whole instruction, which the list deliberately does not", async () => {
    open();
    await waitFor(() =>
      expect((screen.getByLabelText("Instruction") as HTMLTextAreaElement).value).toBe(
        "check what I promised and remind me",
      ),
    );
    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("Boss commitment nudge");
  });

  it("switches a routine off without waiting for a save", async () => {
    // Turning something off is not an edit to what it says. Parking it behind
    // a Save means a routine the operator thought they had stopped still runs.
    open();
    const toggle = await screen.findByRole("switch");
    expect(toggle.getAttribute("aria-checked")).toBe("true");

    fireEvent.click(toggle);
    await waitFor(() => expect(setRoutineActive).toHaveBeenCalledWith("r1", false));
    await waitFor(() =>
      expect(screen.getByRole("switch").getAttribute("aria-checked")).toBe("false"),
    );
    expect(updateRoutine).not.toHaveBeenCalled();
  });

  it("refuses a test run of something other than what is on screen", async () => {
    // The button exists to answer "what does this do". Firing the saved
    // version while the operator is looking at an edited one answers a
    // different question and looks like the edit did nothing.
    open();
    await screen.findByLabelText("Instruction");
    fireEvent.change(screen.getByLabelText("Instruction"), { target: { value: "something else" } });

    expect((screen.getByRole("button", { name: "Test run" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
    expect(testRoutine).not.toHaveBeenCalled();
  });

  it("fires a test run and shows it in the history", async () => {
    open();
    await screen.findByLabelText("Instruction");
    expect(screen.getByText("No runs yet.")).toBeTruthy();

    routineRuns.mockResolvedValue([{ runId: "run-1", kind: "test", at: MORNING }]);
    fireEvent.click(screen.getByRole("button", { name: "Test run" }));

    await waitFor(() => expect(testRoutine).toHaveBeenCalledWith("r1"));
    // Marked as a test: a button press and a real firing look identical in the
    // transcript, so "did it run on Tuesday" has to be answered here.
    expect(await screen.findByText("test run")).toBeTruthy();
  });

  it("does not move the schedule when only the wording changed", async () => {
    open();
    await screen.findByLabelText("Instruction");
    fireEvent.change(screen.getByLabelText("Instruction"), {
      target: { value: "check what I promised and nudge me" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateRoutine).toHaveBeenCalledTimes(1));
    expect(updateRoutine.mock.calls[0]![1].inSecs).toBeNull();
  });

  it("moves the schedule when the time was the thing that changed", async () => {
    open();
    await screen.findByLabelText("Time of day");
    fireEvent.change(screen.getByLabelText("Time of day"), { target: { value: "17:00" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateRoutine).toHaveBeenCalledTimes(1));
    expect(updateRoutine.mock.calls[0]![1].inSecs).toBeGreaterThan(0);
  });

  it("keeps a gap an agent chose for itself in the picker", async () => {
    // `every:18000` is not one of the offered choices. Dropping it would mean
    // saving an unrelated edit silently rewrote the agent's own schedule.
    open({ trigger: "every:18000" });
    await waitFor(() =>
      expect((screen.getByLabelText("Trigger") as HTMLSelectElement).value).toBe("every:18000"),
    );
    // And a gap has no hour to set, so nothing offers to set one.
    expect(screen.queryByLabelText("Time of day")).toBeNull();
  });

  it("asks before deleting, then goes back to the list", async () => {
    const onBack = open();
    await screen.findByLabelText("Instruction");

    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(deleteRoutine).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Delete it" }));
    await waitFor(() => expect(deleteRoutine).toHaveBeenCalledWith("r1"));
    await waitFor(() => expect(onBack).toHaveBeenCalled());
  });

  it("creates a new routine, then behaves like an existing one", async () => {
    const onBack = vi.fn();
    render(<RoutineDetail agentId="a1" routineId="new" onBack={onBack} />);

    // Nothing to switch off or test until it exists.
    expect(screen.queryByRole("switch")).toBeNull();
    expect(screen.queryByRole("button", { name: "Test run" })).toBeNull();

    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Listings sweep" } });
    fireEvent.change(screen.getByLabelText("Instruction"), {
      target: { value: "check the listings" },
    });
    fireEvent.change(screen.getByLabelText("Trigger"), { target: { value: "weekdays" } });
    fireEvent.click(screen.getByRole("button", { name: "Create routine" }));

    await waitFor(() => expect(createRoutine).toHaveBeenCalledTimes(1));
    const [agentId, draft] = createRoutine.mock.calls[0]!;
    expect(agentId).toBe("a1");
    expect(draft.name).toBe("Listings sweep");
    expect(draft.trigger).toBe("weekdays");
    // The time of day reaches the backend as a delay to the first firing.
    expect(draft.inSecs).toBeGreaterThan(0);

    expect(await screen.findByRole("switch")).toBeTruthy();
  });

  it("goes back when the routine has been deleted from under it", async () => {
    // The panel can be looking at a row an agent has since cancelled itself.
    agentRoutines.mockResolvedValue([]);
    const onBack = vi.fn();
    render(<RoutineDetail agentId="a1" routineId="r1" onBack={onBack} />);
    await waitFor(() => expect(onBack).toHaveBeenCalled());
  });
});
