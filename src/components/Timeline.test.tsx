import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import { anAgent, anApproval } from "../test-fixtures";

vi.mock("../lib/ipc", () => ({
  api: {
    upcomingRuns: async () => [],
    calendar: async () => [],
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
