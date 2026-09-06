import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { useStore } from "../lib/store";
import type { WorkDecision } from "../lib/types";
import { aDecision } from "../test-fixtures";
import { ForYou } from "./ForYou";

const mocks = vi.hoisted(() => ({
  list: vi.fn<() => Promise<WorkDecision[]>>(),
  answer: vi.fn(),
  snooze: vi.fn(),
  resume: vi.fn(),
}));
vi.mock("../lib/ipc", () => ({
  api: {
    listDecisions: mocks.list,
    answerDecision: mocks.answer,
    snoozeDecision: mocks.snooze,
    resumeDecision: mocks.resume,
  },
}));

beforeEach(() => {
  vi.resetAllMocks();
  mocks.list.mockResolvedValue([aDecision()]);
  useStore.setState({
    decisions: [aDecision()],
    decisionError: null,
    banner: null,
    pending: [],
    stuck: [],
    agents: [],
    groups: [],
    lastActive: {},
  });
});

it("keeps a submitted question until the runtime confirms it, then tracks follow-through separately", async () => {
  let accept!: () => void;
  mocks.answer.mockImplementation(
    () =>
      new Promise<void>((resolve) => {
        accept = resolve;
      }),
  );
  await act(async () => {
    render(<ForYou onClose={() => {}} />);
  });
  await waitFor(() => expect(mocks.list).toHaveBeenCalled());
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "11 AM" }));
  });
  expect(mocks.answer).toHaveBeenCalledWith("decision-1", "11 AM", 1000);
  expect(screen.getByRole<HTMLButtonElement>("button", { name: "11 AM" }).disabled).toBe(true);
  mocks.list.mockResolvedValue([aDecision({ status: "answered", answer: "11 AM" })]);
  await act(async () => accept());
  await waitFor(() => expect(screen.queryByRole("button", { name: "11 AM" })).toBeNull());
  fireEvent.click(screen.getByRole("button", { name: "Following through 1" }));
  expect(
    screen.getByText("Answer delivered. Awaiting a completion receipt from the agent."),
  ).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "History" }));
  expect(screen.queryByText("10 AM or 11 AM?")).toBeNull();
});

it("shows a failed answer beside the still-answerable question", async () => {
  mocks.answer.mockRejectedValue(new Error("Decision changed; review again."));
  await act(async () => {
    render(<ForYou onClose={() => {}} />);
  });
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "11 AM" }));
  });
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "Decision changed; review again.",
  );
  await waitFor(() =>
    expect(screen.getByRole<HTMLButtonElement>("button", { name: "11 AM" }).disabled).toBe(false),
  );
});

it("allows a written answer even when choices were offered", async () => {
  await act(async () => {
    render(<ForYou onClose={() => {}} />);
  });
  fireEvent.click(screen.getByRole("button", { name: "Write an answer" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Your answer" }), {
    target: { value: "  Neither; try Friday.  " },
  });
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Save answer" }));
  });
  await waitFor(() =>
    expect(mocks.answer).toHaveBeenCalledWith("decision-1", "Neither; try Friday.", 1000),
  );
});

it("keeps interrupted follow-through in Needs you and resumes without asking the choice again", async () => {
  const interrupted = aDecision({ status: "answered", answer: "11 AM", interrupted: true });
  mocks.list.mockResolvedValue([interrupted]);
  useStore.setState({ decisions: [interrupted] });
  await act(async () => {
    render(<ForYou onClose={() => {}} />);
  });
  expect(screen.queryByRole("button", { name: "11 AM" })).toBeNull();
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Resume follow-through" }));
  });
  await waitFor(() => expect(mocks.resume).toHaveBeenCalledWith("decision-1"));
});

it("keeps snoozed decisions discoverable and counts them until answered", async () => {
  const later = aDecision({ snoozedUntil: Date.now() + 3600000 });
  mocks.list.mockResolvedValue([later]);
  useStore.setState({ decisions: [later] });
  await act(async () => {
    render(<ForYou onClose={() => {}} />);
  });
  expect(screen.getByText("Set aside · 1")).toBeTruthy();
  expect(screen.getByText("1 item needs your attention.")).toBeTruthy();
});

it("does not let an older refresh overwrite a newer answer", async () => {
  let old!: (rows: WorkDecision[]) => void;
  mocks.list.mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        old = resolve;
      }),
  );
  const first = useStore.getState().refreshDecisions();
  mocks.list.mockResolvedValue([aDecision({ status: "answered", answer: "11 AM" })]);
  await useStore.getState().refreshDecisions();
  old([aDecision()]);
  await first;
  expect(useStore.getState().decisions[0]?.status).toBe("answered");
});
