import { fireEvent, render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { TaskMap } from "../lib/taskMap";
import type { Envelope } from "../lib/types";
import { ActiveTaskMaps } from "./TaskMap";

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
});
