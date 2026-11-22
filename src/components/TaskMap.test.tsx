import {
  act,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { api } from "../lib/ipc";
import { useStore } from "../lib/store";
import type { TaskMap } from "../lib/taskMap";
import type { Envelope } from "../lib/types";
import { ActiveTaskMaps, useTaskMaps } from "./TaskMap";

const lookups = { byId: () => undefined, byName: () => undefined };

function task(subject: string, deadline: number, count = 1): TaskMap {
  const brief: Envelope = {
    id: subject,
    runId: subject,
    channelId: "lead",
    from: { kind: "human" },
    to: { kind: "agent", id: "lead" },
    parts: [{ type: "text", text: subject }],
    trust: "operator",
    hop: 0,
    intent: "work",
    expectsReply: true,
    cause: null,
    createdAt: 1,
  };
  return {
    brief,
    runId: brief.runId,
    inFlight: true,
    branches: Array.from({ length: count }, (_, index) => ({
      agentId: `branch-${index}`,
      asked: brief,
      reply: null,
      delegatedTo: [],
      failed: null,
      questions: index
        ? []
        : [
            {
              id: subject,
              agentId: "branch-0",
              groupId: "group",
              runId: subject,
              request: { kind: "question", options: ["Yes", "No"] },
              summary: `Question on ${subject}`,
              detail: [],
              state: "pending",
              answer: null,
              createdAt: 1,
              decidedAt: null,
              lapsesAt: deadline,
            },
          ],
    })),
  };
}

describe("pinned task maps", () => {
  it("picks the nearest deadline when two tasks have questions", () => {
    render(
      <ActiveTaskMaps
        maps={[task("First task", 100), task("Second task", 50)]}
        lookups={lookups}
      />,
    );
    expect(screen.getByText("Question on Second task")).toBeTruthy();
    expect(screen.queryByText("Question on First task")).toBeNull();
    expect(screen.getAllByRole("region", { name: "Task map" })).toHaveLength(2);
  });

  it("does not reopen a folded task when the other task settles", () => {
    const first = task("First task", 100);
    const second = task("Second task", 50);
    const { rerender } = render(<ActiveTaskMaps maps={[first, second]} lookups={lookups} />);
    fireEvent.click(screen.getByRole("button", { name: /Hide map/ }));
    rerender(<ActiveTaskMaps maps={[second]} lookups={lookups} />);
    expect(screen.queryByText("Question on Second task")).toBeNull();
    expect(screen.getByRole("button", { name: /Second task.*Show map/ })).toBeTruthy();
  });

  it("fits three branches before offering one control for the rest", () => {
    render(<ActiveTaskMaps maps={[task("Big task", 70, 5)]} lookups={lookups} />);
    const map = within(screen.getByRole("region", { name: "Task map" }));
    expect(map.getAllByRole("listitem")).toHaveLength(3);
    fireEvent.click(map.getByRole("button", { name: "Show 2 more colleagues" }));
    expect(map.getAllByRole("listitem")).toHaveLength(3);
    expect(
      within(screen.getByRole("dialog", { name: "More colleagues" })).getAllByRole("listitem"),
    ).toHaveLength(2);
  });

  it("keeps a reader open through a fold and settlement without reopening it", () => {
    const current = task("First task", 100, 5);
    const sooner = task("Next task", 50);
    const { rerender } = render(<ActiveTaskMaps maps={[current]} lookups={lookups} />);
    fireEvent.click(screen.getByRole("button", { name: "Show 2 more colleagues" }));
    rerender(<ActiveTaskMaps maps={[current, sooner]} lookups={lookups} />);
    expect(document.activeElement).toBe(screen.getByRole("dialog", { name: "More colleagues" }));
    rerender(<ActiveTaskMaps maps={[sooner]} lookups={lookups} />);
    expect(document.activeElement).toBe(screen.getByRole("dialog", { name: "More colleagues" }));
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    rerender(<ActiveTaskMaps maps={[current]} lookups={lookups} />);
    expect(screen.queryByRole("dialog", { name: "More colleagues" })).toBeNull();
  });
});

it("keeps the recorded result when the task is too large to map", async () => {
  const run = task("long-task", 100);
  const dispatch: Envelope = {
    ...run.brief,
    parts: [
      {
        type: "toolCall",
        name: "send_message",
        arguments: {},
        outcome: { status: "ok", summary: "" },
      },
    ],
  };
  const read = vi
    .spyOn(api, "taskRecord")
    .mockResolvedValue({ messages: null, resultId: "result" });
  try {
    const { result } = renderHook(() => useTaskMaps("lead", [dispatch], ""));
    await waitFor(() => expect(result.current.results.get(run.runId)).toBe("result"));
    expect(result.current.maps).toEqual([]);
    expect(result.current.unavailable.get(run.runId)).toBe("oversized");
  } finally {
    read.mockRestore();
  }
});

it("keeps Escape on the waiting question, so Escape then 1 answers it", async () => {
  const answerQuestion = vi.fn(async () => {});
  useStore.setState({ answerQuestion });
  const { container } = render(
    <>
      <ActiveTaskMaps maps={[task("Lisbon", 100)]} lookups={lookups} />
      <textarea aria-label="Write to the lead" data-field="" />
    </>,
  );
  const question = container.querySelector<HTMLElement>(".task-map__question") as HTMLElement;
  question.focus();
  fireEvent.keyDown(question, { key: "Escape" });
  await act(() => new Promise<void>((done) => requestAnimationFrame(() => done())));
  expect(document.activeElement).toBe(question);
  fireEvent.keyDown(document.activeElement as Element, { key: "1" });
  expect(answerQuestion).toHaveBeenCalledExactlyOnceWith("Lisbon", "Yes");
});
