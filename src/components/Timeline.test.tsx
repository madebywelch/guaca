import { act, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { Outcome } from "../lib/types";
import { anAgent, anApproval, anOutcome } from "../test-fixtures";

const { listOutcomes } = vi.hoisted(() => ({
  listOutcomes: vi.fn<(query: { since?: number; limit: number }) => Promise<Outcome[]>>(),
}));

vi.mock("../lib/ipc", () => ({
  api: {
    upcomingRuns: async () => [],
    calendar: async () => [],
    listOutcomes,
  },
}));

const { Timeline } = await import("./Timeline");

const NOW = Date.now();

beforeEach(() => {
  useStore.setState({
    agents: [anAgent("dara", { name: "Dara" })],
    activity: {},
    outcomes: [],
    pending: [],
  });
  listOutcomes.mockResolvedValue([]);
});

describe("an agent that has finished today", () => {
  it("rests on the timeline once its run is seen, though the clock has not ticked since", async () => {
    const finished = Date.now() + 30_000;
    render(<Timeline onOpenAgent={vi.fn()} onOpenGroups={vi.fn()} />);
    expect(await screen.findByRole("button", { name: "Dara, idle" })).toBeTruthy();

    listOutcomes.mockResolvedValue([
      anOutcome({
        agentId: "dara",
        startedAt: finished - 10_000,
        finishedAt: finished,
        seenAt: finished,
      }),
    ]);
    act(() => useStore.setState({ outcomes: [] }));

    expect(await screen.findByRole("button", { name: /^Dara, last active / })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Dara, idle" })).toBeNull();
  });
});

describe("a timeline whose runs for today are still being read", () => {
  it("offers no face until the read lands, so a face is first drawn where it belongs", async () => {
    let land: (rows: Outcome[]) => void = () => {};
    listOutcomes.mockReturnValue(
      new Promise<Outcome[]>((resolve) => {
        land = resolve;
      }),
    );
    render(<Timeline onOpenAgent={vi.fn()} onOpenGroups={vi.fn()} />);
    const list = screen.getByRole("list", { name: "Today's timeline" });
    await act(async () => {});
    expect(listOutcomes).toHaveBeenCalled();
    expect(within(list).queryAllByRole("button")).toEqual([]);
    expect(screen.getByRole("navigation", { name: "Agents" }).getAttribute("aria-busy")).toBe(
      "true",
    );

    const ran = Date.now() - 60_000;
    await act(async () =>
      land([anOutcome({ agentId: "dara", startedAt: ran - 10_000, finishedAt: ran, seenAt: ran })]),
    );
    expect(await within(list).findByRole("button", { name: /^Dara, last active / })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Dara, idle" })).toBeNull();
    expect(screen.getByRole("navigation", { name: "Agents" }).hasAttribute("aria-busy")).toBe(
      false,
    );
  });

  it("draws what it has when the read fails, rather than nothing for good", async () => {
    listOutcomes.mockRejectedValue(new Error("the host went away"));
    vi.spyOn(console, "warn").mockImplementation(() => {});
    render(<Timeline onOpenAgent={vi.fn()} onOpenGroups={vi.fn()} />);
    expect(await screen.findByRole("button", { name: "Dara, idle" })).toBeTruthy();
  });
});

describe("where a parked request stops waiting", () => {
  it("leaves the timeline once the request is filed in For you", async () => {
    useStore.setState({
      pending: [anApproval({ agentId: "dara", createdAt: NOW - 60_000, lapsesAt: NOW + 540_000 })],
    });
    render(<Timeline onOpenAgent={vi.fn()} onOpenGroups={vi.fn()} />);
    expect(await screen.findByText(/^Dara stops waiting/)).toBeTruthy();

    act(() => useStore.setState({ pending: [] }));

    expect(screen.queryByText(/stops waiting/)).toBeNull();
  });
});
