import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Occasion, OccasionDraft } from "../lib/types";
import { aGroup, DEFAULT_GROUP } from "../test-fixtures";

/**
 * The calendar, over a mocked store.
 *
 * The arithmetic behind days, months and labels is `lib/calendar`'s and is
 * tested there. What is worth checking here is the wiring nothing else covers:
 * which window is asked for, what the crew chips actually do to the list, and
 * what a save sends — including the one field that means two things, which is
 * the date.
 */

const calendar = vi.fn<(from: number, until: number) => Promise<Occasion[]>>(async () => []);
const createOccasion = vi.fn<(draft: OccasionDraft) => Promise<Occasion>>();
const updateOccasion = vi.fn<(id: string, draft: OccasionDraft) => Promise<Occasion>>();
const deleteOccasion = vi.fn<(id: string) => Promise<void>>(async () => {});

vi.mock("../lib/ipc", () => ({
  api: {
    calendar: (from: number, until: number) => calendar(from, until),
    createOccasion: (draft: OccasionDraft) => createOccasion(draft),
    updateOccasion: (id: string, draft: OccasionDraft) => updateOccasion(id, draft),
    deleteOccasion: (id: string) => deleteOccasion(id),
  },
}));

const { Calendar } = await import("./Calendar");
const { useStore } = await import("../lib/store");

const OTHER_CREW = "00000000-0000-4000-8000-000000000002";

/** A local wall-clock moment, so every assertion here survives any timezone. */
function at(year: number, month: number, day: number, hour = 0, minute = 0): number {
  return new Date(year, month - 1, day, hour, minute, 0, 0).getTime();
}

let next = 0;
function occasion(over: Partial<Occasion> = {}): Occasion {
  next += 1;
  return {
    id: `occasion-${next}`,
    groupId: DEFAULT_GROUP,
    agentId: null,
    title: "Board call",
    detail: "",
    place: "",
    startsAt: at(2026, 9, 14, 15),
    minutes: 60,
    allDay: false,
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

const onClose = vi.fn();

function open(occasions: Occasion[]) {
  calendar.mockResolvedValue(occasions);
  useStore.setState({
    agents: [],
    groups: [aGroup({ name: "Ops" }), aGroup({ id: OTHER_CREW, name: "Legal" })],
    calendarVersion: 0,
  });
  return render(<Calendar onClose={onClose} />);
}

beforeEach(() => {
  // `Date` alone, so `waitFor` still runs on a real clock. Every window this
  // panel asks for is derived from "now", so a suite that read the real one
  // would assert different months every month it ran.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(at(2026, 9, 10, 9));
  vi.clearAllMocks();
  calendar.mockResolvedValue([]);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("what it asks for", () => {
  it("opens on this month and the rest of the next", async () => {
    // A calendar opened on the 29th and showing only the calendar month is a
    // calendar showing two days.
    open([]);
    await waitFor(() => expect(calendar).toHaveBeenCalled());
    expect(calendar).toHaveBeenCalledWith(at(2026, 9, 1), at(2026, 11, 1));
  });

  it("reads every crew and narrows in the view", async () => {
    // One read rather than one per crew: the chips carry a count each, so the
    // whole set is needed whichever crew is picked, and asking Rust per crew
    // would make the round trips the number of crews.
    open([occasion(), occasion({ title: "Deposition", groupId: OTHER_CREW })]);
    await waitFor(() => expect(screen.getByText("Board call")).toBeTruthy());

    expect(calendar).toHaveBeenCalledTimes(1);
    expect(calendar.mock.calls[0]).toHaveLength(2);
  });

  it("moves a month at a time and comes back", async () => {
    open([]);
    await waitFor(() => expect(calendar).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: /next month/i }));
    await waitFor(() =>
      expect(calendar).toHaveBeenLastCalledWith(at(2026, 10, 1), at(2026, 11, 1)),
    );

    fireEvent.click(screen.getByRole("button", { name: /^today$/i }));
    await waitFor(() => expect(calendar).toHaveBeenLastCalledWith(at(2026, 9, 1), at(2026, 11, 1)));
  });
});

describe("the crews", () => {
  it("shows every crew at once by default", async () => {
    // The whole reason this surface exists. A calendar you have to pick a crew
    // to see is the per-crew framing it was built to escape.
    open([
      occasion({ title: "Board call" }),
      occasion({ title: "Deposition", groupId: OTHER_CREW }),
    ]);

    await waitFor(() => expect(screen.getByText("Board call")).toBeTruthy());
    expect(screen.getByText("Deposition")).toBeTruthy();
  });

  it("narrows to one crew and back", async () => {
    open([
      occasion({ title: "Board call" }),
      occasion({ title: "Deposition", groupId: OTHER_CREW }),
    ]);
    await waitFor(() => expect(screen.getByText("Board call")).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: /^legal/i }));
    expect(screen.queryByText("Board call")).toBeNull();
    expect(screen.getByText("Deposition")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /^all crews/i }));
    expect(screen.getByText("Board call")).toBeTruthy();
  });

  it("offers a crew whose calendar is empty", async () => {
    // A chip that appeared only once somebody wrote a date is a filter you
    // cannot use until you no longer need it.
    open([occasion({ title: "Board call" })]);
    await waitFor(() => expect(screen.getByText("Board call")).toBeTruthy());

    const legal = screen.getByRole("button", { name: /^legal/i });
    expect(legal).toBeTruthy();
    fireEvent.click(legal);
    expect(screen.getByText(/nothing on this calendar for this crew/i)).toBeTruthy();
  });

  it("says which crew every row belongs to, filtered or not", async () => {
    // The crew is why one crew's agents cannot touch a row, so a row that only
    // says it while you happen to be looking at everything hides the thing that
    // matters about it.
    const { container } = open([occasion({ title: "Deposition", groupId: OTHER_CREW })]);
    await waitFor(() => expect(screen.getByText("Deposition")).toBeTruthy());

    expect(container.querySelector(".occasion__crew")?.textContent).toBe("Legal");
    fireEvent.click(screen.getByRole("button", { name: /^legal/i }));
    expect(container.querySelector(".occasion__crew")?.textContent).toBe("Legal");
  });
});

describe("writing one", () => {
  it("sends the date as it was typed, so one field carries both kinds", async () => {
    // A date with a time is a moment and a date without one is a whole day.
    // Rust decides which; a checkbox here would be a second thing to keep in
    // step with the string beside it.
    createOccasion.mockResolvedValue(occasion());
    open([]);
    await waitFor(() => expect(calendar).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: /^add$/i }));
    fireEvent.change(screen.getByLabelText("What is happening"), {
      target: { value: "Q3 filing" },
    });
    fireEvent.change(screen.getByLabelText("When"), { target: { value: "2026-10-15" } });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(createOccasion).toHaveBeenCalled());
    expect(createOccasion.mock.calls[0]![0]).toMatchObject({
      groupId: DEFAULT_GROUP,
      title: "Q3 filing",
      startsAt: "2026-10-15",
      minutes: null,
    });
  });

  it("lands a new one in the crew being looked at", async () => {
    createOccasion.mockResolvedValue(occasion({ groupId: OTHER_CREW }));
    open([]);
    await waitFor(() => expect(calendar).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: /^legal/i }));
    fireEvent.click(screen.getByRole("button", { name: /^add$/i }));
    fireEvent.change(screen.getByLabelText("What is happening"), { target: { value: "Filing" } });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(createOccasion).toHaveBeenCalled());
    expect(createOccasion.mock.calls[0]![0]!.groupId).toBe(OTHER_CREW);
  });

  it("opens an existing one with its date already written the way it is read", async () => {
    open([occasion({ title: "Board call", startsAt: at(2026, 9, 14, 15), minutes: 60 })]);
    await waitFor(() => expect(screen.getByText("Board call")).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: /board call/i }));
    expect((screen.getByLabelText("When") as HTMLInputElement).value).toBe("2026-09-14 15:00");
    expect((screen.getByLabelText("Minutes") as HTMLInputElement).value).toBe("60");
  });

  it("writes a whole day back as a date with no time on it", async () => {
    // Round-tripping an all-day occasion through the editor must not give it a
    // midnight, which would turn a deadline into a 12:00 AM appointment.
    open([
      occasion({ title: "Q3 filing", startsAt: at(2026, 9, 14), allDay: true, minutes: null }),
    ]);
    await waitFor(() => expect(screen.getByText("Q3 filing")).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: /q3 filing/i }));
    expect((screen.getByLabelText("When") as HTMLInputElement).value).toBe("2026-09-14");
  });

  it("will not let an occasion be moved to another crew", async () => {
    // Moving one between crews would move it out from under the agents that
    // keep it, and there is no call that does it.
    open([occasion({ title: "Board call" })]);
    await waitFor(() => expect(screen.getByText("Board call")).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: /board call/i }));
    expect((screen.getByLabelText("Crew") as HTMLSelectElement).disabled).toBe(true);
  });

  it("keeps the editor open when a date could not be read", async () => {
    // The commonest failure is a date Rust refused, and closing the editor
    // would throw away everything typed beside the one field to change.
    createOccasion.mockRejectedValue(new Error("`next tuesday` is not a date this understands"));
    open([]);
    await waitFor(() => expect(calendar).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: /^add$/i }));
    fireEvent.change(screen.getByLabelText("What is happening"), { target: { value: "Call" } });
    fireEvent.change(screen.getByLabelText("When"), { target: { value: "next tuesday" } });
    fireEvent.click(screen.getByRole("button", { name: /^save$/i }));

    await waitFor(() => expect(screen.getByText(/is not a date/i)).toBeTruthy());
    expect((screen.getByLabelText("What is happening") as HTMLInputElement).value).toBe("Call");
  });

  it("cancels one, and only from an occasion that exists", async () => {
    open([occasion({ title: "Board call" })]);
    await waitFor(() => expect(screen.getByText("Board call")).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: /^add$/i }));
    expect(screen.queryByRole("button", { name: /cancel it/i })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: /^back$/i }));
    fireEvent.click(screen.getByRole("button", { name: /board call/i }));
    fireEvent.click(screen.getByRole("button", { name: /cancel it/i }));
    await waitFor(() => expect(deleteOccasion).toHaveBeenCalledWith(`occasion-${next}`));
  });
});

describe("staying current", () => {
  it("re-reads when an agent moves something mid-turn", async () => {
    // Without it the list on screen is the one that was true when the panel
    // opened, and the only way to find out otherwise is to close it and reopen.
    open([]);
    await waitFor(() => expect(calendar).toHaveBeenCalledTimes(1));

    act(() => useStore.getState().applyEvent({ type: "calendarChanged", groupId: DEFAULT_GROUP }));
    await waitFor(() => expect(calendar).toHaveBeenCalledTimes(2));
  });

  it("says what this calendar is not, once, at the bottom", async () => {
    // The app has a Google Calendar plugin and this is not it, which is the one
    // thing about this surface nobody would guess.
    const { container } = open([]);
    await waitFor(() => expect(calendar).toHaveBeenCalled());

    expect(container.querySelector(".calendar__note")?.textContent).toMatch(
      /guaca's own calendar/i,
    );
  });
});
