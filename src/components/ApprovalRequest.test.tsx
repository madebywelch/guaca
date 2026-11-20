import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, ApprovalState, Envelope, Part, WorkDecision } from "../lib/types";
import { MessageItem } from "./MessageItem";

const decideApproval = vi.fn<(id: string, decision: string) => Promise<unknown>>();
const approvalStates = vi.fn(async () => ({}));

vi.mock("../lib/ipc", () => ({
  api: {
    decideApproval: (id: string, decision: string) => decideApproval(id, decision),
    approvalStates: () => approvalStates(),
    approvalAnswers: async () => ({}),
    pendingApprovals: async () => [],
  },
}));

const MANAGER: AgentCard = {
  id: "manager",
  groupId: "g1",
  sandboxId: null,
  browserId: null,
  hasComputer: false,
  hasBrowser: false,
  runsErrands: false,
  browserConsent: "open",
  name: "Manager",
  avatar: "avocado",
  color: "#c7d96b",
  model: "",
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

const REQUEST: Extract<Part, { type: "approval" }> = {
  type: "approval",
  id: "ap1",
  action: "createAgent",
  summary: "Manager wants to create an agent called Chief of Product",
  detail: [
    { label: "Name", value: "Chief of Product" },
    { label: "Instructions", value: "You own the roadmap." },
  ],
};

function asking(part: Extract<Part, { type: "approval" }> = REQUEST): Envelope {
  return {
    id: "m1",
    runId: "r1",
    channelId: MANAGER.id,
    from: { kind: "system" },
    to: { kind: "agent", id: MANAGER.id },
    parts: [part],
    trust: "system",
    hop: 0,
    expectsReply: false,
    intent: "courtesy",
    cause: null,
    createdAt: 0,
  };
}

function filedFor(id: string): WorkDecision {
  return {
    id: "d1",
    agentId: MANAGER.id,
    groupId: "g1",
    topic: `approval:${id}`,
    request: { question: "", context: "", recommendation: "", options: [], source: "" },
    status: "pending",
    answer: null,
    outcome: null,
    createdAt: 0,
    updatedAt: 0,
    dueAt: null,
    remindAt: 0,
    snoozedUntil: null,
    deliveryRun: null,
    interrupted: false,
  };
}

function draw(state: ApprovalState | undefined, part = REQUEST) {
  useStore.setState({ approvals: state ? { [part.id]: state } : {}, banner: null });
  return render(
    <MessageItem
      message={asking(part)}
      lookups={{ byId: (id) => (id === MANAGER.id ? MANAGER : undefined), byName: () => undefined }}
      continued={false}
    />,
  );
}

describe("a request for permission", () => {
  beforeEach(() => {
    decideApproval.mockReset();
    decideApproval.mockResolvedValue(undefined);
    approvalStates.mockClear();
    useStore.setState({ decisions: [] });
  });

  it("shows what was asked for, not just that something was", () => {
    draw("pending");
    expect(screen.getByText(REQUEST.summary)).toBeTruthy();
    expect(screen.getByText("You own the roadmap.")).toBeTruthy();
    expect(screen.getByText(/Manager is waiting on you/)).toBeTruthy();
  });

  it("sends the answer the operator chose", () => {
    draw("pending");
    fireEvent.click(screen.getByRole("button", { name: "Always allow" }));
    expect(decideApproval).toHaveBeenCalledWith("ap1", "alwaysAllow");
  });

  it("says who a standing allow is for", () => {
    draw("pending");
    expect(screen.getByText(/Always allow is for Manager only/)).toBeTruthy();
  });

  it.each([
    ["allow", /You allowed this\./],
    ["alwaysAllow", /said not to ask again/],
    ["deny", /You declined/],
    ["expired", /Nobody answered/],
  ] as const)("draws no buttons once it is %s", (state, said) => {
    draw(state);
    expect(screen.queryByRole("button", { name: "Allow" })).toBeNull();
    expect(screen.getByText(said)).toBeTruthy();
    expect(screen.getByText(REQUEST.summary)).toBeTruthy();
  });

  it("says a request that ran out of time is waiting in For you", () => {
    useStore.setState({ decisions: [filedFor(REQUEST.id)] });
    draw("expired");
    expect(
      screen.getByText("Nobody answered in ten minutes. It is waiting in For you."),
    ).toBeTruthy();
    expect(screen.queryByText(/so nothing happened/)).toBeNull();
  });

  it("keeps saying nothing happened when the lapse filed nothing", () => {
    useStore.setState({ decisions: [filedFor("another")] });
    draw("expired");
    expect(screen.getByText("Nobody answered, so nothing happened.")).toBeTruthy();
    expect(screen.queryByText(/waiting in For you/)).toBeNull();
  });

  it("offers no decision for a request it has never heard of", () => {
    draw(undefined);
    expect(screen.queryByRole("button", { name: "Allow" })).toBeNull();
  });

  it("says a question answered elsewhere is no longer live", () => {
    draw("answered");
    expect(screen.queryByRole("button", { name: "Allow" })).toBeNull();
    expect(screen.getByText(/This question is no longer live\./)).toBeTruthy();
  });

  it("takes the server's answer when a decision is refused", async () => {
    decideApproval.mockRejectedValue({
      kind: "alreadyAnswered",
      message: "that question was already answered (answered)",
    });
    draw("pending");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Deny" }));
    });

    expect(approvalStates).toHaveBeenCalled();
    expect(useStore.getState().banner?.text).toBe("That question was already answered (answered).");
  });

  it("does not offer a standing yes for acting in the operator's name", () => {
    draw("pending", {
      type: "approval",
      id: "ap2",
      action: "actOnBehalf",
      summary: "Outreach wants to do something in your name",
      detail: [
        { label: "What Outreach will do", value: "Email the response to robert@madebywelch.com" },
      ],
    });

    expect(screen.getByRole("button", { name: "Allow" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Deny" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Always allow" })).toBeNull();
    expect(screen.getByText("Nothing here is remembered after this turn.")).toBeTruthy();
    expect(screen.getByText(/Nothing has been sent yet/)).toBeTruthy();
  });
});
