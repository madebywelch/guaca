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

const MORNING = new Date(2025, 5, 10, 9, 28, 0, 0).getTime();

const NOW = MORNING - 3600_000;

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
    webhookAddress.mockResolvedValue({
      url: "http://127.0.0.1:4711/events",
      secret: "s3cr3t-s3cr3t",
    });
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
      { runId: "run-1", kind: "test", at: MORNING, spent: spent({ calls: 2 }), missed: 0 },
    ]);
    fireEvent.click(screen.getByRole("button", { name: "Test run" }));

    await waitFor(() => expect(testRoutine).toHaveBeenCalledWith("r1"));
    expect(await screen.findByText("test run")).toBeTruthy();
  });

  it("says which firings did nothing, and what the others cost", async () => {
    routineRuns.mockResolvedValue([
      { runId: "run-2", kind: "scheduled", at: MORNING, spent: spent({ calls: 0 }), missed: 0 },
      {
        runId: "run-1",
        kind: "scheduled",
        at: MORNING - 86_400_000,
        spent: { prompt: 1200, completion: 300, cost: 0.004, calls: 3 },
        missed: 0,
      },
    ]);
    open();

    expect(await screen.findByText("nothing ran")).toBeTruthy();
    expect(screen.getByText("1.5k")).toBeTruthy();
    expect(screen.getByText("$0.0040")).toBeTruthy();
  });

  it("says which firings were skipped rather than leaving a gap", async () => {
    routineRuns.mockResolvedValue([
      { runId: null, kind: "skipped", at: MORNING, spent: spent({ calls: 0 }), missed: 0 },
      {
        runId: "run-1",
        kind: "scheduled",
        at: MORNING - 86_400_000,
        spent: spent({ calls: 2 }),
        missed: 0,
      },
    ]);
    open({ skipIfWorking: true });

    expect(await screen.findByText("skipped, already working")).toBeTruthy();
    expect(screen.queryByText("nothing ran")).toBeNull();
  });

  it("says how many slots a late firing stands for, and nothing on one that was on time", async () => {
    routineRuns.mockResolvedValue([
      { runId: "run-3", kind: "scheduled", at: MORNING, spent: spent({ calls: 2 }), missed: 3 },
      {
        runId: "run-2",
        kind: "scheduled",
        at: MORNING - 4 * 86_400_000,
        spent: spent({ calls: 2 }),
        missed: 1,
      },
      {
        runId: "run-1",
        kind: "scheduled",
        at: MORNING - 5 * 86_400_000,
        spent: spent({ calls: 2 }),
        missed: 0,
      },
    ]);
    open();

    expect(
      await screen.findByText("Fired late, missed 3 times while the host was off"),
    ).toBeTruthy();
    expect(screen.getByText("Fired late, missed 1 time while the host was off")).toBeTruthy();
    expect(screen.getAllByText(/Fired late/)).toHaveLength(2);
  });

  it("offers the skip only where there is a next firing to fall back on", async () => {
    open({ trigger: "once" });
    await screen.findByLabelText("Date");
    expect(screen.queryByLabelText(/Skip it if the agent/)).toBeNull();
  });

  it("drops the skip when the trigger it depended on becomes a one-off", async () => {
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
    open();
    const tick = (await screen.findByLabelText(/Skip it if the agent/)) as HTMLInputElement;
    expect(tick.checked).toBe(false);

    fireEvent.click(tick);
    expect(updateRoutine).not.toHaveBeenCalled();

    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));
    await waitFor(() => expect(updateRoutine).toHaveBeenCalledTimes(1));
    expect(updateRoutine.mock.calls[0]![1].skipIfWorking).toBe(true);
    expect(updateRoutine.mock.calls[0]![1].inSecs).toBeNull();
  });

  it("asks which weekday a weekly routine keeps", async () => {
    open({ trigger: "weekly" });
    const day = (await screen.findByLabelText("Day of the week")) as HTMLSelectElement;
    expect(day.value).toBe("2");

    fireEvent.change(day, { target: { value: "4" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateRoutine).toHaveBeenCalledTimes(1));
    expect(updateRoutine.mock.calls[0]![1].inSecs).toBe(2 * 86_400 + 3600);
  });

  it("asks which day of the month a monthly routine keeps", async () => {
    open({ trigger: "monthly" });
    const day = (await screen.findByLabelText("Day of the month")) as HTMLSelectElement;
    expect(day.value).toBe("10");
    expect(screen.queryByLabelText("Day of the week")).toBeNull();
  });

  it("takes a date for a one-off rather than only a time of day", async () => {
    open({ trigger: "once" });
    const date = (await screen.findByLabelText("Date")) as HTMLInputElement;
    expect(date.value).toBe("2025-06-10");

    fireEvent.change(date, { target: { value: "2025-06-13" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(updateRoutine).toHaveBeenCalledTimes(1));
    expect(updateRoutine.mock.calls[0]![1].inSecs).toBe(3 * 86_400 + 3600);
  });

  it("refuses to save a one-off whose moment has gone", async () => {
    open({ trigger: "once" });
    fireEvent.change(await screen.findByLabelText("Date"), { target: { value: "2020-01-01" } });

    expect(
      (screen.getByRole("button", { name: "Save changes" }) as HTMLButtonElement).disabled,
    ).toBe(true);
    expect(screen.getByText(/already passed/)).toBeTruthy();
  });

  it("draws a routine waiting on an event as its two names and the address to post to", async () => {
    open({ trigger: "event:stripe/invoice.payment_failed", nextRunAt: null });

    await waitFor(() =>
      expect((screen.getByLabelText("Trigger") as HTMLSelectElement).value).toBe("event:/"),
    );
    expect((screen.getByLabelText("Service") as HTMLInputElement).value).toBe("stripe");
    expect((screen.getByLabelText("Event") as HTMLInputElement).value).toBe(
      "invoice.payment_failed",
    );
    expect(
      await screen.findByText(
        "Nothing on the clock: it fires each time this is posted to Guaca, and Test run delivers it now.",
      ),
    ).toBeTruthy();
    expect(screen.queryByText(/only be posted to a Host/)).toBeNull();
    const line = await screen.findByText(/curl -X POST/);
    expect(line.textContent).toContain(
      "http://127.0.0.1:4711/events/stripe/invoice.payment_failed",
    );
    expect(line.textContent).toContain("Authorization: Bearer s3cr3t-s3cr3t");
    expect(screen.queryByLabelText("Time of day")).toBeNull();
    expect(screen.queryByLabelText("Date")).toBeNull();
    expect(screen.getByRole("button", { name: "Test run" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Save changes" })).toBeNull();
  });

  it("says an event can only be posted to a Host when there is no address", async () => {
    webhookAddress.mockResolvedValue({ secret: "s3cr3t-s3cr3t" });
    open({ trigger: "event:stripe/invoice.payment_failed", nextRunAt: null });

    expect(
      await screen.findByText(
        "Nothing on the clock. An event can only be posted to a Host, so in this workspace the routine fires only from Test run.",
      ),
    ).toBeTruthy();
    expect(screen.queryByText(/receiver is not running/)).toBeNull();
    expect(screen.queryByText(/each time this is posted to Guaca/)).toBeNull();
    expect(screen.queryByText(/curl -X POST/)).toBeNull();
    expect(screen.queryByText(/127\.0\.0\.1/)).toBeNull();
  });

  it("holds a half-named event and creates a whole one", async () => {
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
    expect((await screen.findByText(/curl -X POST/)).textContent).toContain(
      "/events/Stripe/invoice.paid",
    );

    fireEvent.click(screen.getByRole("button", { name: "Create routine" }));
    await waitFor(() => expect(createRoutine).toHaveBeenCalledTimes(1));
    const [, draft] = createRoutine.mock.calls[0]!;
    expect(draft.trigger).toBe("event:Stripe/invoice.paid");
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
    open({ trigger: "every:18000" });
    await waitFor(() =>
      expect((screen.getByLabelText("Trigger") as HTMLSelectElement).value).toBe("every:18000"),
    );
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
    expect(draft.inSecs).toBeGreaterThan(0);

    expect(await screen.findByRole("switch")).toBeTruthy();
  });

  it("goes back when the routine has been deleted from under it", async () => {
    agentRoutines.mockResolvedValue([]);
    const onBack = vi.fn();
    render(<RoutineDetail agentId="a1" routineId="r1" onBack={onBack} />);
    await waitFor(() => expect(onBack).toHaveBeenCalled());
  });
});
