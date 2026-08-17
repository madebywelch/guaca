import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, ApprovalState, Envelope, Part } from "../lib/types";
import { MessageItem } from "./MessageItem";

const decideApproval = vi.fn<(id: string, decision: string) => Promise<unknown>>();
const approvalStates = vi.fn(async () => ({}));

vi.mock("../lib/ipc", () => ({
  api: {
    decideApproval: (id: string, decision: string) => decideApproval(id, decision),
    approvalStates: () => approvalStates(),
  },
}));

const MANAGER: AgentCard = {
  id: "manager",
  groupId: "g1",
  sandboxId: null,
  name: "Manager",
  avatar: "avocado",
  color: "#c7d96b",
  model: "",
  systemPrompt: "",
  skills: [],
  lifecycle: "active",
  pinned: false,
  version: 1,
  createdAt: 0,
  updatedAt: 0,
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

/** The envelope the runtime writes: Guaca, in the asking agent's channel. */
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

function draw(state: ApprovalState | undefined, part = REQUEST) {
  useStore.setState({ approvals: state ? { [part.id]: state } : {}, banner: null });
  return render(
    <MessageItem
      message={asking(part)}
      lookups={{ byId: (id) => (id === MANAGER.id ? MANAGER : undefined), byName: () => undefined }}
      continued={false}
      feed={false}
    />,
  );
}

describe("a request for permission", () => {
  beforeEach(() => {
    decideApproval.mockReset();
    decideApproval.mockResolvedValue(undefined);
    approvalStates.mockClear();
  });

  it("shows what was asked for, not just that something was", () => {
    // The operator is deciding whether an agent should exist. A summary alone
    // would have them approving a name with the instructions out of sight.
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
    // "Always" reads as a workspace setting. It is one agent being let off one
    // question, and the operator has to know that before clicking it.
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

  it("offers no decision for a request it has never heard of", () => {
    // Older than the window the store loads, so nothing is waiting on it.
    // Buttons here would offer an answer that reaches nobody.
    draw(undefined);
    expect(screen.queryByRole("button", { name: "Allow" })).toBeNull();
  });

  it("takes the server's answer when a decision is refused", async () => {
    // The turn timed out while this was on screen, or it was answered in
    // another window. Arguing with the store would leave live buttons on a
    // request that is already settled.
    decideApproval.mockRejectedValue({ kind: "alreadyAnswered", message: "already answered" });
    draw("pending");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Deny" }));
    });

    expect(approvalStates).toHaveBeenCalled();
    expect(useStore.getState().banner?.text).toContain("already answered");
  });

  it("does not offer a standing yes for acting in the operator's name", () => {
    // "Always allow" is scoped to an agent and an action, and this action is
    // "act outside the workspace". A standing yes would cover every future
    // send, submission and purchase rather than the one being asked about.
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
    expect(screen.getByText("This answer covers this one action only.")).toBeTruthy();
    // And the waiting line says what has not happened, which is not a creation.
    expect(screen.getByText(/Nothing has been sent yet/)).toBeTruthy();
  });
});
