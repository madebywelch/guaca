import { render, screen } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import { invoke } from "../lib/transport";
import { anAgent, anApproval } from "../test-fixtures";
import { ConversationRoster } from "./ConversationRoster";
import { Desk } from "./Desk";

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
    case "list_outcomes":
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

it("draws a request from a 0.4.0 host with no lapse, in the roster and the question card", async () => {
  await useStore.getState().refreshApprovals();
  expect(useStore.getState().pending).toHaveLength(1);

  const { container } = render(
    <>
      <ConversationRoster onOpen={vi.fn()} onQuestions={vi.fn()} />
      <Desk onOpenChannel={vi.fn()} />
    </>,
  );
  expect(screen.getByRole("button", { name: "Dara, Has a question" })).toBeTruthy();

  expect(screen.getByText("Send an email as you")).toBeTruthy();
  expect(screen.queryByText(/stops waiting/)).toBeNull();
  expect(screen.queryByText(/waits until/)).toBeNull();
  expect(container.innerHTML).not.toContain("NaN");
});

it("distinguishes an agent who is paused from one who is idle", () => {
  useStore.setState({
    agents: [
      anAgent("dara", { name: "Dara", lifecycle: "paused" }),
      anAgent("jo", { name: "Jo" }),
    ],
  });
  render(<ConversationRoster onOpen={vi.fn()} onQuestions={vi.fn()} />);
  expect(screen.getByRole("button", { name: "Dara, Paused" })).toBeTruthy();
  expect(screen.getByRole("button", { name: "Jo, Idle" })).toBeTruthy();
});
