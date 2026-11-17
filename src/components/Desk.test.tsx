import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, Approval, ApprovalId, Escalation } from "../lib/types";
import { DEFAULT_GROUP } from "../test-fixtures";
import { Desk } from "./Desk";

const decideApproval = vi.fn<(id: string, decision: string) => Promise<Approval>>();
const answerQuestion = vi.fn<(id: string, answer: string) => Promise<Approval>>();
const pendingApprovals = vi.fn<() => Promise<Approval[]>>();
const clearEscalation = vi.fn<(id: string) => Promise<void>>();
const openEscalations = vi.fn<() => Promise<Escalation[]>>();

vi.mock("../lib/ipc", () => ({
  api: {
    decideApproval: (id: string, decision: string) => decideApproval(id, decision),
    answerQuestion: (id: string, answer: string) => answerQuestion(id, answer),
    pendingApprovals: () => pendingApprovals(),
    clearEscalation: (id: string) => clearEscalation(id),
    openEscalations: () => openEscalations(),
    approvalStates: async () => ({}),
    approvalAnswers: async () => ({}),
    channelMessages: async () => [],
    conversationFlow: async () => [],
  },
}));

function agent(name: string): AgentCard {
  return {
    id: name,
    groupId: DEFAULT_GROUP,
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "m",
    subscriptionModel: "",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
  };
}

function request(over: Partial<Approval> = {}): Approval {
  return {
    id: "req-1" as ApprovalId,
    agentId: "Manager",
    groupId: DEFAULT_GROUP,
    runId: "run-1",
    request: { kind: "permission", action: "actOnBehalf" },
    summary: "Send an email as you",
    detail: [{ label: "To", value: "vendor@example.com" }],
    state: "pending",
    answer: null,
    createdAt: 0,
    decidedAt: null,
    lapsesAt: null,
    ...over,
  };
}

function stuckOn(over: Partial<Escalation> = {}): Escalation {
  return {
    id: "esc-1",
    agentId: "Manager",
    groupId: DEFAULT_GROUP,
    runId: "run-1",
    summary: "The deploy needs a key only you have.",
    raisedAt: Date.now() - 2 * 24 * 60 * 60 * 1000,
    saidAt: Date.now(),
    times: 1,
    clearedAt: null,
    ...over,
  };
}

const openChannel = vi.fn<(agent: string) => void>();

function draw(pending: Approval[], stuck: Escalation[] = []) {
  useStore.setState({ agents: [agent("Manager")], pending, stuck, approvals: {}, banner: null });
  return render(<Desk onOpenChannel={openChannel} />);
}

function asks(options: string[], over: Partial<Approval> = {}): Approval {
  return request({
    request: { kind: "question", options },
    summary: "Both vendors clear the bar. Which do you want?",
    detail: [],
    ...over,
  });
}

beforeEach(() => {
  openChannel.mockReset();
  answerQuestion.mockReset();
  answerQuestion.mockResolvedValue(request({ state: "answered" }));
  decideApproval.mockReset();
  decideApproval.mockResolvedValue(request({ state: "allow" }));
  pendingApprovals.mockReset();
  pendingApprovals.mockResolvedValue([]);
  clearEscalation.mockReset();
  clearEscalation.mockResolvedValue(undefined);
  openEscalations.mockReset();
  openEscalations.mockResolvedValue([]);
  useStore.setState({ agents: [], pending: [], stuck: [], approvals: {}, banner: null });
});

describe("the desk", () => {
  it("is not on screen at all when nothing is waiting", () => {
    const { container } = draw([]);
    expect(container.querySelector(".for-you__requests")).toBeNull();
  });

  it("draws a request where it can be answered without going to find it", () => {
    draw([request()]);

    expect(screen.getByText("Send an email as you")).toBeTruthy();
    expect(screen.getByText("vendor@example.com")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Allow" })).toBeTruthy();
  });

  it("draws every word a model wrote as text, never as markup", () => {
    const summary = "**Pay** the <button>vendor</button> [now](https://example.com)";
    const label = "<b>To</b>";
    const value = '<img src="x" alt="sent"> and _all_ of them';
    const options = ["<i>Yes</i>", "**No**"];
    const { container } = draw([
      request({ summary, detail: [{ label, value }] }),
      asks(options, { id: "req-2" as ApprovalId, summary: "# Choose" }),
    ]);

    for (const text of [summary, label, value, "# Choose"]) {
      expect(screen.getByText(text)).toBeTruthy();
    }
    for (const option of options) {
      expect(screen.getByRole("button", { name: option })).toBeTruthy();
    }
    expect(container.querySelectorAll("strong, em, b, i, img, a, h1").length).toBe(0);
    expect(screen.queryByRole("button", { name: "vendor" })).toBeNull();
  });

  it("counts the queue rather than saying there is one", () => {
    draw([request(), request({ id: "req-2" as ApprovalId })]);
    expect(screen.getByText("2 turns are waiting on you")).toBeTruthy();
  });

  it("answers the request it was asked about", () => {
    draw([request({ id: "req-9" as ApprovalId })]);

    fireEvent.click(screen.getByRole("button", { name: "Allow" }));
    expect(decideApproval).toHaveBeenCalledWith("req-9", "allow");
  });

  it("offers no standing yes for anything done in the operator's name", () => {
    draw([request({ request: { kind: "permission", action: "actOnBehalf" } })]);
    expect(screen.queryByRole("button", { name: "Always" })).toBeNull();
  });

  it("offers one for creating an agent, which is narrow enough", () => {
    draw([request({ request: { kind: "permission", action: "createAgent" } })]);
    expect(screen.getByRole("button", { name: "Always" })).toBeTruthy();
  });

  it("stops saying when it lapses the moment that time passes, with nothing else redrawing it", () => {
    vi.useFakeTimers();
    try {
      vi.setSystemTime(new Date(2026, 10, 16, 9, 12));
      draw([request({ lapsesAt: new Date(2026, 10, 16, 9, 13).getTime() })]);
      expect(screen.getByText(/waits until/)).toBeTruthy();
      act(() => {
        vi.advanceTimersByTime(61_000);
      });
      expect(screen.queryByText(/waits until/)).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("opens the channel the request came from", () => {
    draw([request()]);

    fireEvent.click(screen.getByRole("button", { name: "Open channel" }));
    expect(openChannel).toHaveBeenCalledWith("Manager");
  });

  it("names an agent that has since been deleted rather than drawing nothing", () => {
    useStore.setState({ agents: [], pending: [request()] });
    render(<Desk onOpenChannel={openChannel} />);
    expect(screen.getByText("A deleted agent")).toBeTruthy();
  });

  it("shows new requests while an older blocker remains", () => {
    const { rerender } = draw([], [stuckOn()]);
    useStore.setState({ pending: [request()] });
    rerender(<Desk onOpenChannel={openChannel} />);
    expect(screen.getByRole("button", { name: "Allow" })).toBeTruthy();
  });

  it("keeps a card whose answer the runtime refused", async () => {
    decideApproval.mockRejectedValue(new Error("already settled"));
    pendingApprovals.mockResolvedValue([request()]);
    draw([request()]);

    fireEvent.click(screen.getByRole("button", { name: "Allow" }));
    await vi.waitFor(() => expect(useStore.getState().banner?.tone).toBe("error"));
    expect(screen.getByRole("button", { name: "Allow" })).toBeTruthy();
  });

  it("offers a question's own choices, and no verdict", () => {
    draw([asks(["Northwind", "Contoso"])]);

    expect(screen.getByRole("button", { name: "Northwind" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Allow" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Deny" })).toBeNull();
  });

  it("answers with the choice that was pressed", () => {
    draw([asks(["Northwind", "Contoso"], { id: "q-1" as ApprovalId })]);

    fireEvent.click(screen.getByRole("button", { name: "Contoso" }));
    expect(answerQuestion).toHaveBeenCalledWith("q-1", "Contoso");
  });

  it("takes a written answer when the question offered no choices", () => {
    draw([asks([], { id: "q-2" as ApprovalId })]);

    const field = screen.getByPlaceholderText("Your answer");
    fireEvent.change(field, { target: { value: "  Northwind  " } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(answerQuestion).toHaveBeenCalledWith("q-2", "Northwind");
  });

  it("will not send an empty answer", () => {
    draw([asks([])]);

    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(answerQuestion).not.toHaveBeenCalled();
  });

  it("lets the buttons go again after a refusal, so it can be answered", async () => {
    decideApproval.mockRejectedValue(new Error("already settled"));
    pendingApprovals.mockResolvedValue([request()]);
    draw([request()]);

    fireEvent.click(screen.getByRole("button", { name: "Allow" }));
    await vi.waitFor(() =>
      expect(screen.getByRole<HTMLButtonElement>("button", { name: "Allow" }).disabled).toBe(false),
    );
  });
});

describe("an escalation on the desk", () => {
  it("is on it with nothing parked, because nothing has to be parked", () => {
    draw([], [stuckOn()]);
    expect(screen.getByText("The deploy needs a key only you have.")).toBeTruthy();
    expect(screen.getByText("1 agent needs your help")).toBeTruthy();
  });

  it("says how long it has been true and how many turns have hit it", () => {
    draw([], [stuckOn({ times: 6 })]);
    expect(screen.getByText("Manager · stuck 2d")).toBeTruthy();
    expect(screen.getByText("6 turns have run into this")).toBeTruthy();
  });

  it("agrees with itself at one, which is the count it opens with", () => {
    draw([], [stuckOn({ times: 1 })]);
    expect(screen.getByText("1 turn has run into this")).toBeTruthy();
  });

  it("leads with the channel and not with the clear", () => {
    draw([], [stuckOn()]);
    const open = screen.getByRole("button", { name: "Open channel" });
    expect(open.className).toContain("btn--primary");
    expect(screen.getByRole("button", { name: "Clear" }).className).toContain("btn--ghost");
  });

  it("opens the channel of the agent that raised it", () => {
    draw([], [stuckOn()]);
    fireEvent.click(screen.getByRole("button", { name: "Open channel" }));
    expect(openChannel).toHaveBeenCalledWith("Manager");
  });

  it("clears the one it was asked about", () => {
    draw([], [stuckOn({ id: "esc-9" })]);
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(clearEscalation).toHaveBeenCalledWith("esc-9");
  });

  it("reads the queue back after a clear that was refused", async () => {
    clearEscalation.mockRejectedValue(new Error("gone"));
    openEscalations.mockResolvedValue([stuckOn()]);
    draw([], [stuckOn()]);

    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    await vi.waitFor(() => expect(useStore.getState().stuck).toHaveLength(1));
    expect(useStore.getState().banner?.tone).toBe("error");
  });

  it("draws requests above escalations, whatever their ages are", () => {
    const { container } = draw([request()], [stuckOn()]);
    const cards = [...container.querySelectorAll(".desk__card")];
    expect(cards).toHaveLength(2);
    expect(cards[0]?.getAttribute("data-kind")).toBeNull();
    expect(cards[1]?.getAttribute("data-kind")).toBe("stuck");
  });

  it("counts both kinds in one line without adding them up for the operator", () => {
    draw([request()], [stuckOn()]);
    expect(screen.getByText("2 things are waiting on you")).toBeTruthy();
  });

  it("names the agent as deleted rather than drawing an empty card", () => {
    draw([], [stuckOn({ agentId: "gone" })]);
    expect(screen.getByText(/A deleted agent · stuck/)).toBeTruthy();
  });
});
