import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useStore } from "../lib/store";
import type { Outcome, RunId, WorkDecision } from "../lib/types";
import { aDecision, aGroup, anAgent } from "../test-fixtures";
import { ForYou } from "./ForYou";

const mocks = vi.hoisted(() => ({
  list: vi.fn<() => Promise<WorkDecision[]>>(),
  answer: vi.fn(),
  snooze: vi.fn(),
  resume: vi.fn(),
  outcomes: vi.fn<(query: { unseenOnly?: boolean }) => Promise<Outcome[]>>(),
  seen: vi.fn<(target: { runIds?: RunId[]; before?: number }) => Promise<RunId[]>>(),
}));
vi.mock("../lib/ipc", () => ({
  api: {
    listDecisions: mocks.list,
    answerDecision: mocks.answer,
    snoozeDecision: mocks.snooze,
    resumeDecision: mocks.resume,
    listOutcomes: mocks.outcomes,
    markOutcomesSeen: mocks.seen,
  },
}));

const NOW = Date.now();
const HOUR = 3_600_000;

function outcome(runId: string, over: Partial<Outcome> = {}): Outcome {
  return {
    runId,
    agentId: "agent-1",
    groupId: "group-1",
    trigger: "routine",
    triggerRef: null,
    status: "done",
    headline: "Sent the digest to 12 people.",
    noteId: "note-1",
    needs: [],
    steps: 1,
    startedAt: NOW - 3 * HOUR,
    finishedAt: NOW - 2 * HOUR,
    seenAt: null,
    ...over,
  };
}

function hostHas(rows: Outcome[]) {
  useStore.setState({ outcomes: rows.filter((one) => one.seenAt === null) });
  mocks.outcomes.mockImplementation(async (query) =>
    query.unseenOnly ? rows.filter((one) => one.seenAt === null) : rows,
  );
}

beforeEach(() => {
  vi.resetAllMocks();
  mocks.list.mockResolvedValue([aDecision()]);
  mocks.outcomes.mockResolvedValue([]);
  mocks.seen.mockImplementation(async (target) => target.runIds ?? []);
  useStore.setState({
    decisions: [aDecision()],
    decisionError: null,
    banner: null,
    pending: [],
    stuck: [],
    agents: [],
    groups: [],
    lastActive: {},
    outcomes: [],
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
  expect(screen.queryByText(/in progress after your answer/)).toBeNull();
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "11 AM" }));
  });
  expect(mocks.answer).toHaveBeenCalledWith("decision-1", "11 AM", 1000);
  expect(screen.getByRole<HTMLButtonElement>("button", { name: "11 AM" }).disabled).toBe(true);
  mocks.list.mockResolvedValue([aDecision({ status: "answered", answer: "11 AM" })]);
  await act(async () => accept());
  await waitFor(() => expect(screen.queryByRole("button", { name: "11 AM" })).toBeNull());
  expect(
    screen.getByText("Answer saved for the agent. Its progress is under Answered."),
  ).toBeTruthy();
  expect(screen.getByText(/1 in progress after your answer\./)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Answered" }));
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

it("keeps interrupted work in Waiting on you and resumes without asking the choice again", async () => {
  const interrupted = aDecision({ status: "answered", answer: "11 AM", interrupted: true });
  mocks.list.mockResolvedValue([interrupted]);
  useStore.setState({ decisions: [interrupted] });
  await act(async () => {
    render(<ForYou onClose={() => {}} />);
  });
  expect(screen.queryByRole("button", { name: "11 AM" })).toBeNull();
  expect(
    screen.getByText(
      "Work after your answer was interrupted. Review what was already done before resuming.",
    ),
  ).toBeTruthy();
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Resume work" }));
  });
  await waitFor(() => expect(mocks.resume).toHaveBeenCalledWith("decision-1"));
  expect(
    await screen.findByText("Resumed. The agent checks what was already done before going on."),
  ).toBeTruthy();
});

it("sets a question aside for an hour and says it still needs an answer", async () => {
  mocks.snooze.mockResolvedValue(undefined);
  useStore.setState({ decisions: [aDecision()] });
  await act(async () => {
    render(<ForYou onClose={() => {}} />);
  });
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Remind me in 1 hour" }));
  });
  expect(
    await screen.findByText("Set aside for one hour. The question still needs your answer."),
  ).toBeTruthy();
});

it("keeps snoozed decisions discoverable and counts them until answered", async () => {
  const later = aDecision({ snoozedUntil: Date.now() + 3600000 });
  mocks.list.mockResolvedValue([later]);
  useStore.setState({
    decisions: [later],
    agents: [anAgent("assistant", { name: "Executive Assistant" })],
  });
  await act(async () => {
    render(<ForYou onClose={() => {}} />);
  });
  expect(screen.getByText("Set aside · 1")).toBeTruthy();
  expect(screen.getByText("Executive Assistant is waiting on you.")).toBeTruthy();
  expect(screen.getByRole("heading", { level: 2 }).textContent).toBe("For you 1 question");
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

it("opens a request's channel behind For you and gets out of the way", async () => {
  const select = vi.fn(async () => {});
  const close = vi.fn();
  useStore.setState({
    select,
    decisions: [],
    pending: [
      {
        id: "req-1",
        agentId: "agent-1",
        groupId: "group-1",
        runId: "run-1",
        request: { kind: "permission", action: "createAgent" },
        summary: "Wants to hire a scribe",
        detail: [],
        state: "pending",
        answer: null,
        createdAt: 0,
        decidedAt: null,
        lapsesAt: null,
      },
    ],
  });
  mocks.list.mockResolvedValue([]);
  await act(async () => {
    render(<ForYou onClose={close} />);
  });

  fireEvent.click(screen.getByRole("button", { name: "Open conversation" }));
  expect(select).toHaveBeenCalledWith("agent-1");
  expect(close).toHaveBeenCalled();
});

describe("Finished", () => {
  const waiting = {
    id: "req-1",
    agentId: "agent-2",
    groupId: "group-1",
    runId: "run-9",
    request: { kind: "permission" as const, action: "createAgent" as const },
    summary: "Wants to hire a scribe",
    detail: [],
    state: "pending" as const,
    answer: null,
    createdAt: 0,
    decidedAt: null,
    lapsesAt: null,
  };

  beforeEach(() => {
    mocks.list.mockResolvedValue([]);
    useStore.setState({
      decisions: [],
      agents: [],
      groups: [aGroup({ id: "group-1", name: "Research" })],
    });
  });

  it("keeps the empty sentence when nothing needs you and nothing is unseen", async () => {
    await act(async () => {
      render(<ForYou onClose={() => {}} />);
    });
    expect(screen.getByText("Nothing needs your answer right now.")).toBeTruthy();
    expect(screen.queryByRole("region", { name: "Finished" })).toBeNull();
  });

  it("opens on Finished when it is the only thing there", async () => {
    hostHas([outcome("run-1")]);
    await act(async () => {
      render(<ForYou onClose={() => {}} />);
    });
    expect(
      screen.getByText("Nothing needs your answer. 1 finished since you last looked."),
    ).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Waiting on you" }).getAttribute("aria-pressed"),
    ).toBe("true");
    const finished = screen.getByRole("region", { name: "Finished" });
    expect(document.querySelector(".for-you__body")?.firstElementChild).toBe(finished);
    expect(within(finished).getByText("Sent the digest to 12 people.")).toBeTruthy();
  });

  it("sits under the requests and leaves the count to them", async () => {
    hostHas([outcome("run-1"), outcome("run-2", { finishedAt: NOW - HOUR })]);
    useStore.setState({ pending: [waiting], agents: [anAgent("agent-2", { name: "Scribe" })] });
    await act(async () => {
      render(<ForYou onClose={() => {}} />);
    });
    const requests = screen.getByRole("region", { name: "Waiting on you" });
    const finished = screen.getByRole("region", { name: "Finished" });
    expect(
      requests.compareDocumentPosition(finished) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(screen.getByRole("heading", { level: 2 }).textContent).toBe("For you 1 question");
    expect(screen.getByText("Scribe is waiting on you.")).toBeTruthy();
    expect(within(requests).queryByRole("heading")).toBeNull();
  });

  it("opens the closing note in its channel and marks the run seen", async () => {
    const openMessage = vi.fn(async () => {});
    const close = vi.fn();
    useStore.setState({ openMessage });
    hostHas([outcome("run-1", { status: "failed", headline: "Analyst could not reply." })]);
    await act(async () => {
      render(<ForYou onClose={close} />);
    });
    const line = screen.getByRole("button", { name: /Analyst could not reply\./ });
    expect(within(line).getByText("failed 2h")).toBeTruthy();
    await act(async () => {
      fireEvent.click(line);
    });
    expect(mocks.seen).toHaveBeenCalledWith({ runIds: ["run-1"] });
    expect(openMessage).toHaveBeenCalledWith("agent-1", "note-1");
    expect(close).toHaveBeenCalled();
    expect(useStore.getState().outcomes).toEqual([]);
  });

  it("opens the channel itself for a run that left no note", async () => {
    const select = vi.fn(async () => {});
    const openMessage = vi.fn(async () => {});
    useStore.setState({ select, openMessage });
    hostHas([outcome("run-1", { noteId: null, headline: "Finished without a note." })]);
    await act(async () => {
      render(<ForYou onClose={() => {}} />);
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Finished without a note\./ }));
    });
    expect(select).toHaveBeenCalledWith("agent-1");
    expect(openMessage).not.toHaveBeenCalled();
  });

  it("marks everything it drew seen, bounded by the newest line", async () => {
    hostHas([
      outcome("run-1", { finishedAt: NOW - 5 * HOUR }),
      outcome("run-2", { finishedAt: NOW - HOUR }),
    ]);
    await act(async () => {
      render(<ForYou onClose={() => {}} />);
    });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Mark all seen" }));
    });
    expect(mocks.seen).toHaveBeenCalledWith({ before: NOW - HOUR });
  });

  it("heads each group when the lines span more than one", async () => {
    useStore.setState({
      groups: [aGroup({ id: "group-1", name: "Research" }), aGroup({ id: "group-2", name: "Ops" })],
    });
    hostHas([outcome("run-1"), outcome("run-2", { groupId: "group-2", finishedAt: NOW - HOUR })]);
    await act(async () => {
      render(<ForYou onClose={() => {}} />);
    });
    const finished = screen.getByRole("region", { name: "Finished" });
    expect(
      within(finished)
        .getAllByRole("heading", { level: 4 })
        .map((h) => h.textContent),
    ).toEqual(["Ops", "Research"]);
  });

  it("goes back to Waiting on you when asked to open while open on another view", async () => {
    hostHas([outcome("run-1")]);
    useStore.setState({ forYou: true });
    await act(async () => {
      render(<ForYou onClose={() => {}} />);
    });
    fireEvent.click(screen.getByRole("button", { name: "History" }));
    expect(screen.queryByRole("region", { name: "Finished" })).toBeNull();

    await act(async () => {
      useStore.getState().showForYou(true);
    });
    expect(
      screen.getByRole("button", { name: "Waiting on you" }).getAttribute("aria-pressed"),
    ).toBe("true");
    expect(screen.getByRole("region", { name: "Finished" })).toBeTruthy();
  });

  it("keeps what was seen this week one click away under Earlier", async () => {
    hostHas([
      outcome("run-1", { seenAt: NOW - HOUR, headline: "Booked the venue." }),
      outcome("run-2", { seenAt: NOW, finishedAt: NOW - 8 * 24 * HOUR, headline: "Too old." }),
    ]);
    await act(async () => {
      render(<ForYou onClose={() => {}} />);
    });
    await waitFor(() => expect(screen.getByText("Earlier · 1")).toBeTruthy());
    expect(screen.getByText("Booked the venue.")).toBeTruthy();
    expect(screen.queryByText("Too old.")).toBeNull();
    expect(screen.getByText("Nothing needs your answer right now.")).toBeTruthy();
  });
});

it("calls a waiting request a question in every tab, footer and section it has", async () => {
  mocks.list.mockResolvedValue([]);
  useStore.setState({
    decisions: [],
    pending: [],
    stuck: [],
    agents: [anAgent("assistant", { name: "Executive Assistant" })],
  });
  await act(async () => {
    render(<ForYou onClose={() => {}} />);
  });
  const tabs = screen.getByRole("navigation", { name: "For you views" });
  expect(
    within(tabs)
      .getAllByRole("button")
      .map((tab) => tab.textContent),
  ).toEqual(["Waiting on you", "Answered", "History"]);
  expect(screen.getByRole("heading", { level: 2 }).textContent).toBe("For you ");
  expect(screen.getByText("When each agent last worked")).toBeTruthy();
  expect(screen.getByRole("contentinfo").textContent).toMatch(
    /^Reminders about open questions at 9 AM and 4 PM/,
  );
  fireEvent.click(within(tabs).getByRole("button", { name: "Answered" }));
  expect(screen.getByText("Nothing you answered is still in progress.")).toBeTruthy();
  fireEvent.click(within(tabs).getByRole("button", { name: "History" }));
  expect(screen.getByText("Completed and withdrawn questions will appear here.")).toBeTruthy();
});

it("says a question it could not refresh is a question", async () => {
  useStore.setState({ decisions: [], decisionError: "offline" });
  mocks.list.mockRejectedValue(new Error("offline"));
  await act(async () => {
    render(<ForYou onClose={() => {}} />);
  });
  expect(screen.getByRole("alert").textContent).toMatch(/^Could not refresh questions: offline/);
});
