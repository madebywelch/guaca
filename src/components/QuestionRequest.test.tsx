import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, ApprovalState, Envelope, Part } from "../lib/types";
import { MessageItem } from "./MessageItem";

const answerQuestion = vi.fn<(id: string, answer: string) => Promise<unknown>>();

vi.mock("../lib/ipc", () => ({
  api: {
    answerQuestion: (id: string, answer: string) => answerQuestion(id, answer),
    decideApproval: async () => undefined,
    approvalStates: async () => ({}),
    approvalAnswers: async () => ({}),
    pendingApprovals: async () => [],
  },
}));

const ANALYST: AgentCard = {
  id: "analyst",
  groupId: "g1",
  sandboxId: null,
  browserId: null,
  hasComputer: false,
  hasBrowser: false,
  runsErrands: false,
  browserConsent: "open",
  name: "Analyst",
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

type QuestionPart = Extract<Part, { type: "question" }>;

const CHOICE: QuestionPart = {
  type: "question",
  id: "q1",
  question: "Both vendors clear the bar on price. Which do you want?",
  options: ["Northwind", "Contoso"],
};

const WRITTEN: QuestionPart = {
  type: "question",
  id: "q2",
  question: "What budget should I plan against?",
  options: [],
};

function asked(part: QuestionPart): Envelope {
  return {
    id: "m1",
    runId: "r1",
    channelId: ANALYST.id,
    from: { kind: "system" },
    to: { kind: "agent", id: ANALYST.id },
    parts: [part],
    trust: "system",
    hop: 0,
    expectsReply: false,
    intent: "courtesy",
    cause: null,
    createdAt: 0,
  };
}

function draw(
  state: ApprovalState | undefined,
  part = CHOICE,
  answers: Record<string, string> = {},
) {
  useStore.setState({
    approvals: state ? { [part.id]: state } : {},
    answers,
    banner: null,
  });
  return render(
    <MessageItem
      message={asked(part)}
      lookups={{ byId: (id) => (id === ANALYST.id ? ANALYST : undefined), byName: () => undefined }}
      continued={false}
    />,
  );
}

describe("a question put to the operator", () => {
  beforeEach(() => {
    answerQuestion.mockReset();
    answerQuestion.mockResolvedValue(undefined);
  });

  it("asks the question and offers the agent's own choices", () => {
    draw("pending");
    expect(screen.getByText(CHOICE.question)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Northwind" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Contoso" })).toBeTruthy();
  });

  // The whole line between this card and the one beside it. An operator who
  // reads "Allow" here would be answering a question that grants nothing with
  // a word that means it granted something.
  it("offers no verdict, because there is nothing here to permit", () => {
    draw("pending");
    expect(screen.queryByRole("button", { name: "Allow" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Deny" })).toBeNull();
    expect(screen.getByText(/permits nothing/)).toBeTruthy();
  });

  it("sends the choice that was pressed", () => {
    draw("pending");
    fireEvent.click(screen.getByRole("button", { name: "Contoso" }));
    expect(answerQuestion).toHaveBeenCalledWith("q1", "Contoso");
  });

  it("takes a written answer when no choices were offered", () => {
    draw("pending", WRITTEN);

    fireEvent.change(screen.getByPlaceholderText("Your answer"), {
      target: { value: " 40k " },
    });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    expect(answerQuestion).toHaveBeenCalledWith("q2", "40k");
  });

  it("will not send an empty answer", () => {
    draw("pending", WRITTEN);
    fireEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(answerQuestion).not.toHaveBeenCalled();
  });

  // A question that has been answered is history, and history with live
  // buttons on it is an answer that reaches nobody.
  it("stops offering answers once it has one", () => {
    draw("answered");
    expect(screen.queryByRole("button", { name: "Northwind" })).toBeNull();
    expect(screen.getByText("You answered this.")).toBeTruthy();
  });

  // The agent did not stop working: it was told nobody answered and carried on.
  // Saying "nothing happened" here, as the permission card does, would be the
  // opposite of what happened.
  it("says the work went ahead when nobody answered", () => {
    draw("expired");
    expect(screen.getByText(/went ahead without you/)).toBeTruthy();
  });

  it("is not live at all for a question older than the window the store loads", () => {
    draw(undefined);
    expect(screen.queryByRole("button", { name: "Northwind" })).toBeNull();
  });
});

describe("a question that has been answered", () => {
  beforeEach(() => {
    answerQuestion.mockReset();
    answerQuestion.mockResolvedValue(undefined);
  });

  // What the channel is for. The answer reached the agent either way; what was
  // missing is the record of it, and a week later "You answered this" is the
  // one line nobody can reconstruct from anything on screen.
  it("keeps what was said, not only that something was", () => {
    const { container } = draw("answered", CHOICE, { q1: "Contoso" });

    expect(screen.getByText("You answered this.")).toBeTruthy();
    expect(screen.getByText("Contoso")).toBeTruthy();
    expect(container.querySelector(".ask__given")?.textContent).toBe("Contoso");
  });

  it("keeps a written answer whole, including the line breaks", () => {
    const written = "Forty thousand.\nNot a cent more before Q3.";
    draw("answered", WRITTEN, { q2: written });
    expect(screen.getByText(/Forty thousand\./)).toBeTruthy();
  });

  // A decision older than the window the store reads has a state and no answer
  // with it. The card must still say the one thing that is known, and must not
  // draw an empty answer as though the operator had said nothing.
  it("says only that it was answered when no answer came with it", () => {
    const { container } = draw("answered");

    expect(screen.getByText("You answered this.")).toBeTruthy();
    expect(container.querySelector(".ask__given")).toBeNull();
  });

  // A choice is one of the agent's own options, so it can contain anything the
  // model can write. It reaches the card as a value, never as markup.
  it("draws the answer as text, whatever the model put in the option", () => {
    const { container } = draw("answered", CHOICE, { q1: "[Contoso](https://example.com)" });

    expect(screen.getByText("[Contoso](https://example.com)")).toBeTruthy();
    expect(container.querySelector("a, strong, em")).toBeNull();
  });
});
