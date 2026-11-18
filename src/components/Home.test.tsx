import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { HOLD_MS, useHeld } from "../lib/held";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import { clockLabel } from "../lib/time";
import type { Outcome, RunId, Running, WorkDecision } from "../lib/types";
import { aDecision, aGroup, anAgent, anApproval } from "../test-fixtures";
import { Home } from "./Home";

vi.mock("../lib/ipc", () => ({
  api: {
    listOutcomes: vi.fn(async () => []),
    stopRun: vi.fn(async () => true),
    sendMessage: vi.fn(async () => "run"),
  },
}));

const dara = anAgent("agent-1", { name: "Dara" });
const NOW = new Date(2026, 10, 16, 9, 12).getTime();
const ASKED = NOW - 7 * 60_000;
const LAPSES = ASKED + 10 * 60_000;

function draw(state: {
  pending?: ReturnType<typeof anApproval>[];
  decisions?: WorkDecision[];
  running?: Running[];
  outcomes?: Outcome[];
}) {
  act(() => {
    useStore.setState({
      agents: [dara],
      groups: [],
      pending: state.pending ?? [],
      stuck: [],
      decisions: state.decisions ?? [],
      outcomes: state.outcomes ?? [],
      running: state.running ?? [],
      activity: {},
      activeRun: {},
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

describe("running work", () => {
  const run: Running = {
    runId: "run-1" as RunId,
    agentId: dara.id,
    groupId: dara.groupId,
    startedAt: NOW - 125_000,
    note: "Reading the quarterly filings",
  };

  it("shows who is working, their latest note and how long it has run, from the host's start", () => {
    draw({ running: [run] });
    const section = screen.getByRole("region", { name: /Running/ });
    expect(within(section).getByText("Dara")).toBeTruthy();
    expect(within(section).getByText("Reading the quarterly filings")).toBeTruthy();
    expect(within(section).getByText("2m 05s")).toBeTruthy();
  });

  it("stops the conversation it names", () => {
    draw({ running: [run] });
    fireEvent.click(screen.getByRole("button", { name: "Stop Dara's conversation" }));
    expect(api.stopRun).toHaveBeenCalledWith("run-1");
  });

  it("says who the agent is waiting for when it wrote no note", () => {
    const noor = anAgent("agent-2", { name: "Noor" });
    draw({ running: [{ ...run, note: null }] });
    act(() => {
      useStore.setState({
        agents: [dara, noor],
        activeRun: { [noor.id]: run.runId },
      });
    });
    expect(screen.getByText("Waiting for Noor")).toBeTruthy();
  });

  it("sits between what needs you and what finished", () => {
    const outcome: Outcome = {
      runId: "run-0" as RunId,
      agentId: dara.id,
      groupId: dara.groupId,
      trigger: "operator",
      triggerRef: null,
      status: "done",
      headline: "Sent the digest.",
      noteId: null,
      needs: [],
      steps: 1,
      startedAt: NOW - 600_000,
      finishedAt: NOW - 60_000,
      seenAt: null,
    };
    draw({
      pending: [anApproval({ agentId: "agent-1", createdAt: ASKED, lapsesAt: LAPSES })],
      running: [run],
      outcomes: [outcome],
    });
    const headings = screen.getAllByRole("heading").map((one) => one.textContent ?? "");
    const at = (word: string) => headings.findIndex((text) => text.startsWith(word));
    expect(at("Needs you")).toBeGreaterThanOrEqual(0);
    expect(at("Needs you")).toBeLessThan(at("Running"));
    expect(at("Running")).toBeLessThan(at("Finished"));
  });

  it("keeps the calm sentence and lists the work under it, saying it once", () => {
    draw({ running: [run] });
    act(() => useStore.setState({ activity: { [dara.id]: { state: "thinking" } } }));
    expect(screen.getByText("Nothing needs you.")).toBeTruthy();
    expect(screen.queryByText(/Dara is working/)).toBeNull();
    expect(screen.getByRole("region", { name: /Running/ })).toBeTruthy();
  });
});

describe("the field", () => {
  const noor = anAgent("agent-2", { name: "Noor" });

  function drawField(agents = [dara, noor], pending: ReturnType<typeof anApproval>[] = []) {
    act(() => {
      useStore.setState({
        agents,
        groups: [],
        pending,
        stuck: [],
        decisions: [],
        outcomes: [],
        activity: {},
        lastActive: {},
        railGroup: null,
        refreshOutcomes: async () => {},
      });
    });
    const onSearch = vi.fn();
    render(<Home onSearch={onSearch} onManageGroups={vi.fn()} />);
    const input = screen.getByRole("textbox", { name: "What do you need?" }) as HTMLTextAreaElement;
    return { input, onSearch };
  }

  beforeEach(() => {
    vi.mocked(api.sendMessage).mockClear();
  });

  it("hands what is written to search on the find shortcut", () => {
    const { input, onSearch } = drawField();
    fireEvent.change(input, { target: { value: "  the measure fix " } });
    fireEvent.keyDown(input, { key: "f", ctrlKey: true });
    expect(onSearch).toHaveBeenCalledWith("the measure fix");
  });

  it("leaves the find shortcut to the window when nothing is written", () => {
    const { input, onSearch } = drawField();
    expect(fireEvent.keyDown(input, { key: "f", ctrlKey: true })).toBe(true);
    expect(onSearch).not.toHaveBeenCalled();
  });

  it("lets Tab leave the field when something is written, even with a question waiting", () => {
    const asked = anApproval({
      agentId: "agent-1",
      createdAt: ASKED,
      request: { kind: "question", options: [] },
    });
    const { input } = drawField([dara, noor], [asked]);
    fireEvent.change(input, { target: { value: "draft it" } });
    expect(fireEvent.keyDown(input, { key: "Tab" })).toBe(true);
    expect(screen.getByRole("textbox", { name: "What do you need?" })).toBe(input);
    expect(input.value).toBe("draft it");
  });

  it("takes an @Name with nothing after it as a pick, and sends nothing", async () => {
    const { input } = drawField();
    expect(screen.getByRole("button", { name: "Writing to Dara. Change who" })).toBeTruthy();
    fireEvent.change(input, { target: { value: "@Noor" } });
    await act(async () => {
      fireEvent.keyDown(input, { key: "Enter" });
    });
    expect(api.sendMessage).not.toHaveBeenCalled();
    expect(input.value).toBe("");
    expect(screen.getByRole("button", { name: "Writing to Noor. Change who" })).toBeTruthy();
  });

  it("moves the recipient to whoever an @ names as soon as it names them", () => {
    const { input } = drawField();
    expect(screen.getByRole("button", { name: "Writing to Dara. Change who" })).toBeTruthy();
    fireEvent.change(input, { target: { value: "@No" } });
    expect(screen.getByRole("button", { name: "Writing to Dara. Change who" })).toBeTruthy();
    fireEvent.change(input, { target: { value: "@Noor" } });
    expect(screen.getByRole("button", { name: "Writing to Noor. Change who" })).toBeTruthy();
    fireEvent.change(input, { target: { value: "@Noor check the" } });
    expect(screen.getByRole("button", { name: "Writing to Noor. Change who" })).toBeTruthy();
    fireEvent.change(input, { target: { value: "check the" } });
    expect(screen.getByRole("button", { name: "Writing to Dara. Change who" })).toBeTruthy();
  });

  it("sends nothing and says why when there are no agents", async () => {
    const { input } = drawField([]);
    expect(screen.getByRole("button", { name: "Choose who to write to" })).toBeTruthy();
    expect(screen.getByText("Nothing needs you.")).toBeTruthy();
    fireEvent.change(input, { target: { value: "hello" } });
    await act(async () => {
      fireEvent.keyDown(input, { key: "Enter" });
    });
    expect(api.sendMessage).not.toHaveBeenCalled();
    expect(screen.getByRole("alert").textContent).toBe(
      "There is nobody to write to yet. Add an agent first.",
    );
    expect(input.value).toBe("hello");
  });
});

describe("the field's recipient picker", () => {
  const noor = anAgent("agent-2", { name: "Noor" });

  function open() {
    act(() => {
      useStore.setState({
        agents: [dara, noor],
        groups: [aGroup()],
        pending: [],
        stuck: [],
        decisions: [],
        outcomes: [],
        activity: {},
        lastActive: {},
        railGroup: null,
        refreshOutcomes: async () => {},
      });
    });
    render(<Home onSearch={vi.fn()} onManageGroups={vi.fn()} />);
    const trigger = screen.getByRole("button", { name: "Writing to Dara. Change who" });
    fireEvent.click(trigger);
    return { trigger, menu: screen.getByRole("group", { name: "Write to" }) };
  }

  it("is buttons, not a listbox that ignores the arrows", () => {
    open();
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.queryByRole("option")).toBeNull();
  });

  it("opens on whoever is written to and moves with the arrows", () => {
    const { menu } = open();
    const dara = within(menu).getByRole("button", { name: "Dara" });
    expect(document.activeElement).toBe(dara);
    fireEvent.keyDown(dara, { key: "ArrowDown" });
    expect(document.activeElement).toBe(within(menu).getByRole("button", { name: "Noor" }));
    fireEvent.keyDown(document.activeElement as Element, { key: "ArrowDown" });
    expect(document.activeElement).toBe(
      within(menu).getByRole("button", { name: "Groups and agents" }),
    );
    fireEvent.keyDown(document.activeElement as Element, { key: "ArrowDown" });
    expect(document.activeElement).toBe(dara);
    fireEvent.keyDown(dara, { key: "ArrowUp" });
    expect(document.activeElement).toBe(
      within(menu).getByRole("button", { name: "Groups and agents" }),
    );
  });

  it("closes on Escape and gives focus back to its button", () => {
    const { trigger } = open();
    fireEvent.keyDown(document.activeElement as Element, { key: "Escape" });
    expect(screen.queryByRole("group", { name: "Write to" })).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });

  it("closes on a press anywhere else", () => {
    open();
    fireEvent.pointerDown(document.body);
    expect(screen.queryByRole("group", { name: "Write to" })).toBeNull();
  });
});

describe("a request whose time has passed", () => {
  it("shows no time while it is still pending", () => {
    draw({
      pending: [
        anApproval({ agentId: "agent-1", createdAt: NOW - 20 * 60_000, lapsesAt: NOW - 60_000 }),
      ],
    });
    expect(screen.queryByText(/waits until/)).toBeNull();
  });
});

describe("Needs you as a keyboard queue", () => {
  const noor = anAgent("agent-2", { name: "Noor" });
  const permission = anApproval({ agentId: "agent-1", summary: "Change your name to Robin?" });
  const question = anApproval({
    id: "req-noor",
    agentId: "agent-2",
    summary: "Which story leads Friday?",
    request: { kind: "question", options: ["Notebooks", "The renewal"] },
  });
  const decide = vi.fn(async () => {});
  const answer = vi.fn(async () => {});

  function drawQueue(pending = [permission, question]) {
    act(() => {
      useHeld.setState({ held: null, notice: null });
      useStore.setState({
        agents: [dara, noor],
        groups: [],
        pending,
        stuck: [],
        decisions: [],
        outcomes: [],
        activity: {},
        lastActive: {},
        railGroup: null,
        refreshOutcomes: async () => {},
        decideApproval: decide,
        answerQuestion: answer,
      });
    });
    render(<Home onSearch={vi.fn()} onManageGroups={vi.fn()} />);
    return screen.getByRole("textbox", { name: "What do you need?" });
  }

  const card = (name: RegExp) => screen.getByRole("article", { name });

  beforeEach(() => {
    vi.useFakeTimers({
      now: NOW,
      toFake: ["Date", "setTimeout", "clearTimeout", "requestAnimationFrame"],
    });
    decide.mockClear();
    answer.mockClear();
  });

  it("selects the first card on Escape from the field, and J and K move the selection", () => {
    const field = drawQueue();
    fireEvent.keyDown(field, { key: "Escape" });
    expect(document.activeElement).toBe(card(/^Dara asks/));
    expect(card(/^Dara asks/).hasAttribute("data-selected")).toBe(true);
    fireEvent.keyDown(document.activeElement as Element, { key: "j" });
    expect(document.activeElement).toBe(card(/^Noor asks/));
    expect(card(/^Dara asks/).hasAttribute("data-selected")).toBe(false);
    fireEvent.keyDown(document.activeElement as Element, { key: "k" });
    expect(document.activeElement).toBe(card(/^Dara asks/));
  });

  it("draws the keys on the selected card's buttons and on no other card", () => {
    const field = drawQueue();
    fireEvent.keyDown(field, { key: "Escape" });
    const selected = card(/^Dara asks/);
    expect(within(selected).getByRole("button", { name: "Allow" }).textContent).toMatch(/↩|Enter/);
    expect(within(card(/^Noor asks/)).getByRole("button", { name: "Notebooks" }).textContent).toBe(
      "Notebooks",
    );
    fireEvent.keyDown(selected, { key: "j" });
    expect(within(card(/^Noor asks/)).getByRole("button", { name: "Notebooks" }).textContent).toBe(
      "Notebooks1",
    );
  });

  it("holds an Allow for the Undo's few seconds, moves on, and only then sends it", () => {
    const field = drawQueue();
    fireEvent.keyDown(field, { key: "Escape" });
    fireEvent.keyDown(document.activeElement as Element, { key: "Enter", metaKey: true });
    expect(decide).not.toHaveBeenCalled();
    expect(screen.queryByRole("article", { name: /^Dara asks/ })).toBeNull();
    expect(document.activeElement).toBe(card(/^Noor asks/));
    expect(useHeld.getState().notice?.text).toBe("Allowed Dara's request.");
    act(() => vi.advanceTimersByTime(HOLD_MS));
    expect(decide).toHaveBeenCalledExactlyOnceWith(permission.id, "allow");
  });

  it("sends nothing at all when the answer is undone in time, and puts the card back", () => {
    const field = drawQueue();
    fireEvent.keyDown(field, { key: "Escape" });
    fireEvent.keyDown(document.activeElement as Element, { key: "Backspace", ctrlKey: true });
    act(() => {
      useHeld.getState().undo();
    });
    act(() => vi.advanceTimersByTime(HOLD_MS * 2));
    expect(decide).not.toHaveBeenCalled();
    expect(card(/^Dara asks/)).toBeTruthy();
  });

  it("answers a question with its number, and gives the field back after the last card", () => {
    const field = drawQueue([question]);
    fireEvent.keyDown(field, { key: "Escape" });
    fireEvent.keyDown(document.activeElement as Element, { key: "2" });
    act(() => vi.advanceTimersByTime(HOLD_MS));
    expect(answer).toHaveBeenCalledExactlyOnceWith("req-noor", "The renewal");
    act(() => vi.advanceTimersByTime(16));
    expect(document.activeElement).toBe(screen.getByRole("textbox", { name: "What do you need?" }));
  });

  it("sends a held answer at once when the next one is given, so two answers never wait on one Undo", () => {
    const field = drawQueue();
    fireEvent.keyDown(field, { key: "Escape" });
    fireEvent.keyDown(document.activeElement as Element, { key: "Enter", metaKey: true });
    fireEvent.keyDown(document.activeElement as Element, { key: "1" });
    expect(decide).toHaveBeenCalledExactlyOnceWith(permission.id, "allow");
    expect(answer).not.toHaveBeenCalled();
  });
});
