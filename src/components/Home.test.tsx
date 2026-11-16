import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import { clockLabel } from "../lib/time";
import type { WorkDecision } from "../lib/types";
import { aDecision, anAgent, anApproval } from "../test-fixtures";
import { Home } from "./Home";

vi.mock("../lib/ipc", () => ({
  api: {
    listOutcomes: vi.fn(async () => []),
    sendMessage: vi.fn(async () => "run"),
  },
}));

const dara = anAgent("agent-1", { name: "Dara" });
const NOW = new Date(2026, 10, 16, 9, 12).getTime();
const ASKED = NOW - 7 * 60_000;
const LAPSES = ASKED + 10 * 60_000;

function draw(state: { pending?: ReturnType<typeof anApproval>[]; decisions?: WorkDecision[] }) {
  act(() => {
    useStore.setState({
      agents: [dara],
      groups: [],
      pending: state.pending ?? [],
      stuck: [],
      decisions: state.decisions ?? [],
      outcomes: [],
      activity: {},
      refreshOutcomes: async () => {},
    });
  });
  return render(<Home onSearch={vi.fn()} onManageGroups={vi.fn()} />);
}

beforeEach(() => {
  vi.useFakeTimers({ now: NOW, toFake: ["Date"] });
});

describe("a request on the opening screen", () => {
  it("says when the agent stops waiting, from the time the host sent", () => {
    draw({ pending: [anApproval({ agentId: "agent-1", createdAt: ASKED, lapsesAt: LAPSES })] });
    expect(screen.getByText(`Dara waits until ${clockLabel(LAPSES)}`)).toBeTruthy();
  });

  it("promises no time when the host sent none", () => {
    draw({ pending: [anApproval({ agentId: "agent-1", createdAt: ASKED, lapsesAt: null })] });
    expect(screen.queryByText(/waits until/)).toBeNull();
  });

  it("once filed to For you, says the agent went on, and shows no time", () => {
    const filed = aDecision({
      agentId: "agent-1",
      status: "pending",
      snoozedUntil: null,
      request: {
        question: "Review the measure fix now, or wait for Monday's build?",
        context: "",
        recommendation: "",
        options: [],
        source: "",
      },
    });
    draw({ decisions: [filed] });
    expect(
      screen.getByText("Review the measure fix now, or wait for Monday's build?"),
    ).toBeTruthy();
    expect(
      screen.getByText(
        "Dara went on without an answer. Your answer reaches Dara as a new message.",
      ),
    ).toBeTruthy();
    expect(screen.queryByText(/waits until/)).toBeNull();
    expect(screen.getByRole("button", { name: "Answer in For you" })).toBeTruthy();
    expect(screen.queryByText("Nothing needs you.")).toBeNull();
  });

  it("leaves nothing behind when a request expires without being filed", () => {
    draw({});
    expect(screen.getByText("Nothing needs you.")).toBeTruthy();
    expect(screen.queryByRole("article")).toBeNull();
    expect(screen.queryByText(/waits until|went on without/)).toBeNull();
  });
});

describe("the quiet screen", () => {
  it("is not quiet while a decision is due, and is once it is set aside", () => {
    const later = aDecision({
      agentId: "agent-1",
      status: "pending",
      snoozedUntil: NOW + 3_600_000,
    });
    draw({ decisions: [later] });
    expect(screen.getByText("Nothing needs you.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "1 decision set aside in For you" })).toBeTruthy();
  });
});
