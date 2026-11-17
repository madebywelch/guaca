import { act, render, screen } from "@testing-library/react";
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
    render(<Timeline onOpenAgent={vi.fn()} onOpenPile={vi.fn()} />);
    expect(await screen.findByRole("button", { name: "1 asleep" })).toBeTruthy();

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
    expect(screen.queryByRole("button", { name: "1 asleep" })).toBeNull();
  });
});

describe("where a parked request stops waiting", () => {
  it("leaves the timeline once the request is filed in For you", async () => {
    useStore.setState({
      pending: [anApproval({ agentId: "dara", createdAt: NOW - 60_000, lapsesAt: NOW + 540_000 })],
    });
    render(<Timeline onOpenAgent={vi.fn()} onOpenPile={vi.fn()} />);
    expect(screen.getByText(/^Dara stops waiting/)).toBeTruthy();

    act(() => useStore.setState({ pending: [] }));

    expect(screen.queryByText(/stops waiting/)).toBeNull();
  });
});
