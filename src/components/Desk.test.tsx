import { fireEvent, render, screen } from "@testing-library/react";
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
    repositoryId: null,
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "m",
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
    ...over,
  };
}

/** An escalation, raised a fortnight ago unless a test says otherwise. */
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

function draw(pending: Approval[], stuck: Escalation[] = []) {
  useStore.setState({ agents: [agent("Manager")], pending, stuck, approvals: {}, banner: null });
  return render(<Desk />);
}

/** A question, which is answered with a value rather than with a verdict. */
function asks(options: string[], over: Partial<Approval> = {}): Approval {
  return request({
    request: { kind: "question", options },
    summary: "Both vendors clear the bar. Which do you want?",
    detail: [],
    ...over,
  });
}

beforeEach(() => {
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
  // Being gone almost all the time is what buys the corner of the screen. A
  // panel that is always there is furniture inside a week.
  it("is not on screen at all when nothing is waiting", () => {
    const { container } = draw([]);
    expect(container.querySelector(".desk")).toBeNull();
  });

  it("draws a request where it can be answered without going to find it", () => {
    draw([request()]);

    expect(screen.getByText("Send an email as you")).toBeTruthy();
    expect(screen.getByText("vendor@example.com")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Allow" })).toBeTruthy();
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

  // The same refusal the transcript's card makes, for the same reason: a
  // standing yes to acting in the operator's name would cover every future
  // send rather than this one.
  it("offers no standing yes for anything done in the operator's name", () => {
    draw([request({ request: { kind: "permission", action: "actOnBehalf" } })]);
    expect(screen.queryByRole("button", { name: "Always" })).toBeNull();
  });

  it("offers one for creating an agent, which is narrow enough", () => {
    draw([request({ request: { kind: "permission", action: "createAgent" } })]);
    expect(screen.getByRole("button", { name: "Always" })).toBeTruthy();
  });

  // A summary is enough to answer most requests and not all of them. The way
  // into the conversation around one has to be on the card, or the desk becomes
  // a surface that asks for decisions it cannot support.
  it("opens the channel the request came from", () => {
    draw([request()]);

    fireEvent.click(screen.getByRole("button", { name: "Open channel" }));
    expect(useStore.getState().selected).toBe("Manager");
  });

  it("names an agent that has since been deleted rather than drawing nothing", () => {
    useStore.setState({ agents: [], pending: [request()] });
    render(<Desk />);
    expect(screen.getByText("A deleted agent")).toBeTruthy();
  });

  it("collapses to its count, and still says how many", () => {
    draw([request()]);

    fireEvent.click(screen.getByRole("button", { expanded: true }));
    expect(screen.queryByRole("button", { name: "Allow" })).toBeNull();
    expect(screen.getByText("1 turn is waiting on you")).toBeTruthy();
  });

  it("closes on Escape, being the last thing on screen with a claim to it", () => {
    draw([request()]);

    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("button", { name: "Allow" })).toBeNull();
  });

  // Collapsing is about the requests that were on screen at the time. A desk
  // that stayed shut once it had been emptied would silently hold the next
  // thing that stops work.
  it("opens again for a queue that emptied and refilled", () => {
    const { rerender } = draw([request()]);
    fireEvent.click(screen.getByRole("button", { expanded: true }));

    useStore.setState({ pending: [] });
    rerender(<Desk />);
    useStore.setState({ pending: [request({ id: "req-2" as ApprovalId })] });
    rerender(<Desk />);

    expect(screen.getByRole("button", { name: "Allow" })).toBeTruthy();
  });

  // Nothing about a decision is believed until the runtime has been read again.
  // A card that vanished on click would hide a request that is still live when
  // the answer was refused for landing a moment too late.
  it("keeps a card whose answer the runtime refused", async () => {
    decideApproval.mockRejectedValue(new Error("already settled"));
    pendingApprovals.mockResolvedValue([request()]);
    draw([request()]);

    fireEvent.click(screen.getByRole("button", { name: "Allow" }));
    await vi.waitFor(() => expect(useStore.getState().banner?.tone).toBe("error"));
    expect(screen.getByRole("button", { name: "Allow" })).toBeTruthy();
  });

  // The two kinds are answered differently and both land here. Allow and Deny
  // on "which vendor" would settle the row saying nothing, and the turn would
  // resume having been told nothing at all.
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

  // An empty answer settles the request with nothing in it and the agent
  // resumes as though it had been told something.
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
    expect(screen.getByText("1 agent is stuck on you")).toBeTruthy();
  });

  // The two numbers the message in a channel could not carry. "Stuck" is a
  // state; "stuck for two days, six turns into it" is a decision.
  it("says how long it has been true and how many turns have hit it", () => {
    draw([], [stuckOn({ times: 6 })]);
    expect(screen.getByText("Manager · stuck 2d")).toBeTruthy();
    expect(screen.getByText("6 turns have run into this")).toBeTruthy();
  });

  // Opening the channel is what actually unblocks the agent. Clearing only
  // takes the row away, so it must not be the loud button.
  it("leads with the channel and not with the clear", () => {
    draw([], [stuckOn()]);
    const open = screen.getByRole("button", { name: "Open channel" });
    expect(open.className).toContain("btn--primary");
    expect(screen.getByRole("button", { name: "Clear" }).className).toContain("btn--ghost");
  });

  it("opens the channel of the agent that raised it", () => {
    draw([], [stuckOn()]);
    fireEvent.click(screen.getByRole("button", { name: "Open channel" }));
    expect(useStore.getState().selected).toBe("Manager");
  });

  it("clears the one it was asked about", () => {
    draw([], [stuckOn({ id: "esc-9" })]);
    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    expect(clearEscalation).toHaveBeenCalledWith("esc-9");
  });

  // A clear that failed must not leave the desk missing a row the store still
  // has: the read is what decides, exactly as it does for a refused verdict.
  it("reads the queue back after a clear that was refused", async () => {
    clearEscalation.mockRejectedValue(new Error("gone"));
    openEscalations.mockResolvedValue([stuckOn()]);
    draw([], [stuckOn()]);

    fireEvent.click(screen.getByRole("button", { name: "Clear" }));
    await vi.waitFor(() => expect(useStore.getState().stuck).toHaveLength(1));
    expect(useStore.getState().banner?.tone).toBe("error");
  });

  // One of these has ten minutes to be answered in and the other has as long as
  // it takes. Sorting the perishable half under the durable half is how a
  // permission lapses while the operator reads about a two-day-old wall.
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
