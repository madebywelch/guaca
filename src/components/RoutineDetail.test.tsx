import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Routine, RoutineDraft, RoutineRun, WebhookAddress } from "../lib/types";
import { RoutineDetail } from "./RoutineDetail";

const agentRoutines = vi.fn<() => Promise<Routine[]>>();
const routineRuns = vi.fn<() => Promise<RoutineRun[]>>();
const createRoutine = vi.fn<(agentId: string, draft: RoutineDraft) => Promise<Routine>>();
const updateRoutine = vi.fn<(id: string, draft: RoutineDraft) => Promise<Routine>>();
const setRoutineActive = vi.fn<(id: string, active: boolean) => Promise<Routine>>();
const testRoutine = vi.fn<(id: string) => Promise<string>>();
const deleteRoutine = vi.fn<(id: string) => Promise<void>>();
const webhookAddress = vi.fn<() => Promise<WebhookAddress>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentRoutines: () => agentRoutines(),
    routineRuns: () => routineRuns(),
    createRoutine: (agentId: string, draft: RoutineDraft) => createRoutine(agentId, draft),
    updateRoutine: (id: string, draft: RoutineDraft) => updateRoutine(id, draft),
    setRoutineActive: (id: string, active: boolean) => setRoutineActive(id, active),
    testRoutine: (id: string) => testRoutine(id),
    deleteRoutine: (id: string) => deleteRoutine(id),
    webhookAddress: () => webhookAddress(),
  },
}));

/** 2025-06-10 at 09:28 local, a Tuesday. The slot every routine here holds. */
const MORNING = new Date(2025, 5, 10, 9, 28, 0, 0).getTime();

/**
 * An hour before that slot, and where the clock is pinned.
 *
 * Every delay this panel sends is counted from now, so the assertions about
 * what reaches the backend are only readable against a fixed now. Only `Date`
 * is faked: `waitFor` and the panel's own clock need real timers.
 */
const NOW = MORNING - 3600_000;

/** A firing's spend. Zero calls is the case the history exists to show. */
function spent(over: Partial<RoutineRun["spent"]> = {}): RoutineRun["spent"] {
  return { prompt: 0, completion: 0, cost: null, calls: 1, ...over };
}

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

function open(over: Partial<Routine> = {}) {
  agentRoutines.mockResolvedValue([routine(over)]);
  const onBack = vi.fn();
  render(<RoutineDetail agentId="a1" routineId="r1" onBack={onBack} />);
  return onBack;
}

describe("RoutineDetail", () => {
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(NOW);
    vi.clearAllMocks();
    agentRoutines.mockResolvedValue([routine()]);
    routineRuns.mockResolvedValue([]);
    createRoutine.mockResolvedValue(routine());
    updateRoutine.mockResolvedValue(routine());
    setRoutineActive.mockImplementation(async (_id, active) => routine({ active }));
    testRoutine.mockResolvedValue("run-1");
    deleteRoutine.mockResolvedValue(undefined);
    webhookAddress.mockResolvedValue({ port: 4711, secret: "s3cr3t-s3cr3t" });
  });

  afterEach(() => {
    vi.useRealTimers();
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

    routineRuns.mockResolvedValue([
      { runId: "run-1", kind: "test", at: MORNING, spent: spent({ calls: 2 }) },
    ]);
    fireEvent.click(screen.getByRole("button", { name: "Test run" }));

    await waitFor(() => expect(testRoutine).toHaveBeenCalledWith("r1"));
    // Marked as a test: a button press and a real firing look identical in the
    // transcript, so "did it run on Tuesday" has to be answered here.
    expect(await screen.findByText("test run")).toBeTruthy();
  });

  it("says which firings did nothing, and what the others cost", async () => {
    // A delivered routine whose agent never ran looks exactly like one that
    // worked, and the operator's next move is not the same.
    routineRuns.mockResolvedValue([
      { runId: "run-2", kind: "scheduled", at: MORNING, spent: spent({ calls: 0 }) },
      {
        runId: "run-1",
        kind: "scheduled",
        at: MORNING - 86_400_000,
        spent: { prompt: 1200, completion: 300, cost: 0.004, calls: 3 },
      },
    ]);
    open();

    expect(await screen.findByText("nothing ran")).toBeTruthy();
    expect(screen.getByText("1.5k")).toBeTruthy();
    // Four places, because a firing costs fractions of a cent and $0.00 reads
    // as free.
    expect(screen.getByText("$0.0040")).toBeTruthy();
  });

  it("says which firings were skipped rather than leaving a gap", async () => {
    // A skipped firing that left no row would read as a scheduler that has
    // stopped working, and it must not be drawn as "nothing ran" either: that
    // one was delivered to an agent that did not act on it, which is a
    // different problem with a different fix.
    routineRuns.mockResolvedValue([
      { runId: null, kind: "skipped", at: MORNING, spent: spent({ calls: 0 }) },
      { runId: "run-1", kind: "scheduled", at: MORNING - 86_400_000, spent: spent({ calls: 2 }) },
    ]);
    open({ skipIfWorking: true });

    expect(await screen.findByText("skipped, already working")).toBeTruthy();
    expect(screen.queryByText("nothing ran")).toBeNull();
  });

  it("offers the skip only where there is a next firing to fall back on", async () => {
    // A one-off that skipped would be a one-off that never happened: the slot
    // it was holding is the only one it has. The backend refuses the pair, so
    // the tick must not be reachable there.
    open({ trigger: "once" });
    await screen.findByLabelText("Date");
    expect(screen.queryByLabelText(/Skip it if the agent/)).toBeNull();
  });

  it("drops the skip when the trigger it depended on becomes a one-off", async () => {
    // The tick is hidden on a one-off, so a routine switched to one while it
    // was ticked would be refused by the backend with its reason attached to a
    // control the operator can no longer see.
    open({ trigger: "daily", skipIfWorking: true });
    const tick = (await screen.findByLabelText(/Skip it if the agent/)) as HTMLInputElement;
    expect(tick.checked).toBe(true);

    fireEvent.change(screen.getByLabelText("Trigger"), { target: { value: "once" } });
    fireEvent.change(screen.getByLabelText("Date"), { target: { value: "2025-06-13" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateRoutine).toHaveBeenCalledTimes(1));
    expect(updateRoutine.mock.calls[0]![1].skipIfWorking).toBe(false);
  });

  it("saves the skip as part of the draft rather than on the click", async () => {
    // Unlike Active, which acts at once. This one changes what a firing does
    // rather than whether the routine runs at all, so it belongs with the
    // wording and the schedule, behind Save.
    open();
    const tick = (await screen.findByLabelText(/Skip it if the agent/)) as HTMLInputElement;
    expect(tick.checked).toBe(false);

    fireEvent.click(tick);
    expect(updateRoutine).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(updateRoutine).toHaveBeenCalledTimes(1));
    expect(updateRoutine.mock.calls[0]![1].skipIfWorking).toBe(true);
    // And nothing else moved: ticking it is not a statement about when it runs.
    expect(updateRoutine.mock.calls[0]![1].inSecs).toBeNull();
  });

  it("asks which weekday a weekly routine keeps", async () => {
    // The weekday is not stored anywhere but the first firing, so a picker
    // that cannot say it means "every week" lands on whichever day the
    // operator happened to be at the keyboard.
    open({ trigger: "weekly" });
    const day = (await screen.findByLabelText("Day of the week")) as HTMLSelectElement;
    // 2025-06-10 is a Tuesday, and that is the day this routine is holding.
    expect(day.value).toBe("2");

    fireEvent.change(day, { target: { value: "4" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateRoutine).toHaveBeenCalledTimes(1));
    // Thursday is two days after Tuesday, and the hour it kept is unchanged.
    expect(updateRoutine.mock.calls[0]![1].inSecs).toBe(2 * 86_400 + 3600);
  });

  it("asks which day of the month a monthly routine keeps", async () => {
    open({ trigger: "monthly" });
    const day = (await screen.findByLabelText("Day of the month")) as HTMLSelectElement;
    expect(day.value).toBe("10");
    expect(screen.queryByLabelText("Day of the week")).toBeNull();
  });

  it("takes a date for a one-off rather than only a time of day", async () => {
    // A time alone could only mean the next 24 hours: "remind me on the 3rd"
    // had no way to be said at all.
    open({ trigger: "once" });
    const date = (await screen.findByLabelText("Date")) as HTMLInputElement;
    expect(date.value).toBe("2025-06-10");

    fireEvent.change(date, { target: { value: "2025-06-13" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateRoutine).toHaveBeenCalledTimes(1));
    expect(updateRoutine.mock.calls[0]![1].inSecs).toBe(3 * 86_400 + 3600);
  });

  it("refuses to save a one-off whose moment has gone", async () => {
    // A delay in the past reaches the scheduler as overdue and fires at once,
    // which is not what picking a date means.
    open({ trigger: "once" });
    fireEvent.change(await screen.findByLabelText("Date"), { target: { value: "2020-01-01" } });

    expect(
      (screen.getByRole("button", { name: "Save changes" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(screen.getByText(/already passed/)).toBeTruthy();
  });

  it("draws a routine waiting on an event as its two names and the address to post to", async () => {
    // It must not claim a moment it does not hold, and it has to hand the
    // operator the one thing they need from this screen: the line that fires
    // it, with the secret in the header.
    open({ trigger: "event:stripe/invoice.payment_failed", nextRunAt: null });

    await waitFor(() =>
      expect((screen.getByLabelText("Trigger") as HTMLSelectElement).value).toBe("event:/"),
    );
    expect((screen.getByLabelText("Service") as HTMLInputElement).value).toBe("stripe");
    expect((screen.getByLabelText("Event") as HTMLInputElement).value).toBe(
      "invoice.payment_failed",
    );
    expect(screen.getByText(/Nothing on the clock/)).toBeTruthy();
    const line = await screen.findByText(/curl -X POST/);
    expect(line.textContent).toContain(
      "http://127.0.0.1:4711/events/stripe/invoice.payment_failed",
    );
    expect(line.textContent).toContain("Authorization: Bearer s3cr3t-s3cr3t");
    // No moment to state, so nothing offers to state one.
    expect(screen.queryByLabelText("Time of day")).toBeNull();
    expect(screen.queryByLabelText("Date")).toBeNull();
    // And Test run is still there, and still needs nothing posted.
    expect(screen.getByRole("button", { name: "Test run" })).toBeTruthy();
    // Nothing about reading it was an edit.
    expect(screen.queryByRole("button", { name: "Save changes" })).toBeNull();
  });

  it("says when the receiver is not running rather than printing a dead address", async () => {
    webhookAddress.mockResolvedValue({ port: 0, secret: "s3cr3t-s3cr3t" });
    open({ trigger: "event:stripe/invoice.payment_failed", nextRunAt: null });

    expect(await screen.findByText(/receiver is not running/)).toBeTruthy();
    expect(screen.queryByText(/curl -X POST/)).toBeNull();
  });

  it("holds a half-named event and creates a whole one", async () => {
    // The picker says "an event"; the two names say which. Until both are
    // typed the backend would refuse the spec, so the panel refuses first and
    // says what it is waiting for.
    render(<RoutineDetail agentId="a1" routineId="new" onBack={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Instruction"), {
      target: { value: "chase the invoice" },
    });
    fireEvent.change(screen.getByLabelText("Trigger"), { target: { value: "event:/" } });

    expect(await screen.findByLabelText("Service")).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: "Create routine" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(screen.getByText(/Name the service and what it reports/)).toBeTruthy();
    expect(screen.queryByText(/curl -X POST/)).toBeNull();

    fireEvent.change(screen.getByLabelText("Service"), { target: { value: "Stripe" } });
    fireEvent.change(screen.getByLabelText("Event"), { target: { value: "invoice.paid" } });
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Create routine" }) as HTMLButtonElement).disabled,
      ).toBe(false),
    );
    // Named, the address is drawn for the names as typed.
    expect((await screen.findByText(/curl -X POST/)).textContent).toContain(
      "/events/Stripe/invoice.paid",
    );

    fireEvent.click(screen.getByRole("button", { name: "Create routine" }));
    await waitFor(() => expect(createRoutine).toHaveBeenCalledTimes(1));
    const [, draft] = createRoutine.mock.calls[0]!;
    expect(draft.trigger).toBe("event:Stripe/invoice.paid");
    // No moment to send: an event has no start time, and the backend refuses
    // one.
    expect(draft.inSecs).toBeNull();
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

  it("moves the schedule when the moment was the thing that changed", async () => {
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
    // The panel can be looking at a row an agent has since canceled itself.
    agentRoutines.mockResolvedValue([]);
    const onBack = vi.fn();
    render(<RoutineDetail agentId="a1" routineId="r1" onBack={onBack} />);
    await waitFor(() => expect(onBack).toHaveBeenCalled());
  });
});
