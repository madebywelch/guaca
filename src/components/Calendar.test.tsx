import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Occasion, OccasionDraft } from "../lib/types";
import { aGroup, DEFAULT_GROUP } from "../test-fixtures";

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
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(at(2026, 9, 10, 9));
  vi.clearAllMocks();
  calendar.mockResolvedValue([]);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("what it asks for", () => {
  it("shows the empty state and error when a read fails, then clears it on refresh", async () => {
    open([]);
    await waitFor(() => expect(calendar).toHaveBeenCalledTimes(1));
    calendar.mockRejectedValueOnce(new Error("Calendar unavailable"));
    fireEvent.click(screen.getByRole("button", { name: /next month/i }));
    expect(await screen.findByText("Calendar unavailable")).toBeTruthy();
    expect(screen.getByText(/nothing on this calendar/i)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /^today$/i }));
    await waitFor(() => expect(screen.queryByText("Calendar unavailable")).toBeNull());
  });

  it("keeps the previous data while a month read is in flight", async () => {
    open([occasion()]);
    expect(await screen.findByText("Board call")).toBeTruthy();
    let finish!: (occasions: Occasion[]) => void;
    calendar.mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: /next month/i }));
    expect(screen.queryByText(/loading/i)).toBeNull();
    await act(async () => finish([]));
    expect(screen.getByText(/nothing on this calendar/i)).toBeTruthy();
  });

  it("opens on this month and the rest of the next", async () => {
    open([]);
    await waitFor(() => expect(calendar).toHaveBeenCalled());
    expect(calendar).toHaveBeenCalledWith(at(2026, 9, 1), at(2026, 11, 1));
  });

  it("reads every crew and narrows in the view", async () => {
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

    fireEvent.click(screen.getByRole("button", { name: /^all groups/i }));
    expect(screen.getByText("Board call")).toBeTruthy();
  });

  it("offers a crew whose calendar is empty", async () => {
    open([occasion({ title: "Board call" })]);
    await waitFor(() => expect(screen.getByText("Board call")).toBeTruthy());

    const legal = screen.getByRole("button", { name: /^legal/i });
    expect(legal).toBeTruthy();
    fireEvent.click(legal);
    expect(screen.getByText(/nothing on this calendar for this group/i)).toBeTruthy();
  });

  it("says which crew every row belongs to, filtered or not", async () => {
    const { container } = open([occasion({ title: "Deposition", groupId: OTHER_CREW })]);
    await waitFor(() => expect(screen.getByText("Deposition")).toBeTruthy());

    expect(container.querySelector(".occasion__crew")?.textContent).toBe("Legal");
    fireEvent.click(screen.getByRole("button", { name: /^legal/i }));
    expect(container.querySelector(".occasion__crew")?.textContent).toBe("Legal");
  });
});

describe("writing one", () => {
  it("sends the date as it was typed, so one field carries both kinds", async () => {
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
    open([
      occasion({ title: "Q3 filing", startsAt: at(2026, 9, 14), allDay: true, minutes: null }),
    ]);
    await waitFor(() => expect(screen.getByText("Q3 filing")).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: /q3 filing/i }));
    expect((screen.getByLabelText("When") as HTMLInputElement).value).toBe("2026-09-14");
  });

  it("will not let an occasion be moved to another crew", async () => {
    open([occasion({ title: "Board call" })]);
    await waitFor(() => expect(screen.getByText("Board call")).toBeTruthy());

    fireEvent.click(screen.getByRole("button", { name: /board call/i }));
    expect((screen.getByLabelText("Group") as HTMLSelectElement).disabled).toBe(true);
  });

  it("keeps the editor open when a date could not be read", async () => {
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
    open([]);
    await waitFor(() => expect(calendar).toHaveBeenCalledTimes(1));

    act(() => useStore.getState().applyEvent({ type: "calendarChanged", groupId: DEFAULT_GROUP }));
    await waitFor(() => expect(calendar).toHaveBeenCalledTimes(2));
  });

  it("says what this calendar is not, once, at the bottom", async () => {
    const { container } = open([]);
    await waitFor(() => expect(calendar).toHaveBeenCalled());

    expect(container.querySelector(".calendar__note")?.textContent).toMatch(
      /guaca's own calendar/i,
    );
  });
});
