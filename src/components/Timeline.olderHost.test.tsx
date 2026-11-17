import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import { invoke } from "../lib/transport";
import { anAgent, anApproval } from "../test-fixtures";
import { Desk } from "./Desk";
import { Timeline } from "./Timeline";

vi.mock("../lib/transport", async (original) => ({
  ...(await original<typeof import("../lib/transport")>()),
  invoke: vi.fn(),
}));

function olderHost(name: string): Promise<unknown> {
  switch (name) {
    case "pending_approvals": {
      const { lapsesAt: _, ...older } = anApproval({
        agentId: "dara",
        createdAt: Date.now() - 60_000,
      });
      return Promise.resolve([older]);
    }
    case "upcoming_runs":
      return Promise.reject({
        kind: "unknownCommand",
        message:
          "this build has no command called `upcoming_runs`. This app is Guaca 0.5.0 and the host is 0.4.0; update the host",
      });
    case "calendar":
      return Promise.resolve([]);
    default:
      return Promise.resolve({});
  }
}

beforeEach(() => {
  vi.mocked(invoke).mockImplementation((name) => olderHost(name) as Promise<never>);
  vi.spyOn(console, "error");
  vi.spyOn(console, "warn").mockImplementation(() => {});
  useStore.setState({
    agents: [anAgent("dara", { name: "Dara" })],
    activity: {},
    outcomes: [],
    pending: [],
    stuck: [],
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

it("draws a request from a 0.4.0 host with no lapse, on the timeline or its card", async () => {
  await useStore.getState().refreshApprovals();
  expect(useStore.getState().pending).toHaveLength(1);

  const { container } = render(
    <>
      <Timeline onOpenAgent={vi.fn()} onOpenPile={vi.fn()} />
      <Desk onOpenChannel={vi.fn()} />
    </>,
  );
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("upcoming_runs", expect.anything()));

  expect(screen.getByText("Send an email as you")).toBeTruthy();
  expect(screen.queryByText(/stops waiting/)).toBeNull();
  expect(screen.queryByText(/waits until/)).toBeNull();
  expect(container.innerHTML).not.toContain("NaN");
});

it("shows no schedule when the host has no upcoming_runs, and tells nobody but the console", async () => {
  render(<Timeline onOpenAgent={vi.fn()} onOpenPile={vi.fn()} />);
  await waitFor(() =>
    expect(console.warn).toHaveBeenCalledWith(
      expect.stringMatching(/^Upcoming runs could not be read/),
    ),
  );
  expect(screen.getByRole("list", { name: "Today's timeline" })).toBeTruthy();
  expect(screen.queryByText(/upcoming_runs|could not be read/)).toBeNull();
  expect(console.error).not.toHaveBeenCalled();
});
