import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AgentCard, Repository } from "../lib/types";
import { RailRepositories } from "./RailRepositories";

const GROUP = "00000000-0000-4000-8000-000000000001";

function member(id: string, name: string, repositoryId: string | null = null): AgentCard {
  return {
    id,
    groupId: GROUP,
    name,
    avatar: "avocado",
    color: "#7fb069",
    model: "",
    systemPrompt: "",
    skills: [],
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    repositoryId,
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
  };
}

function repository(over: Partial<Repository> = {}): Repository {
  return {
    id: "r1",
    groupId: GROUP,
    name: "guaca",
    path: "/Users/you/dev/guaca",
    note: "",
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

/** The rail's own row, standing in for it. */
const row = (agent: AgentCard) => <div key={agent.id}>{agent.name}</div>;

function draw(repositories: Repository[], crew: AgentCard[], dragging = false) {
  const onDragOver = vi.fn();
  const rendered = render(
    <RailRepositories
      repositories={repositories}
      crew={crew}
      row={row}
      isOver={() => false}
      onDragOver={onDragOver}
      onDragLeave={vi.fn()}
      dragging={dragging}
    />,
  );
  return { onDragOver, ...rendered };
}

describe("RailRepositories", () => {
  it("draws nothing when a crew has no repositories", () => {
    // Furniture that is always there is furniture, and a crew with no codebase
    // is most crews. The rail is an agent list first.
    const { container } = draw([], [member("a1", "Ada")]);
    expect(container.firstChild).toBeNull();
  });

  it("puts an agent under the repository it works in", () => {
    draw([repository()], [member("a1", "Ada", "r1")]);
    expect(screen.getByText("guaca")).toBeTruthy();
    expect(screen.getByText("Ada").closest(".rail__repo-crew")).toBeTruthy();
  });

  it("draws each agent once, which is what the tree is for", () => {
    // A many-to-many cannot be a tree. The exclusive rule is what buys every
    // agent exactly one place in the rail, and this is that property.
    draw(
      [repository({ id: "r1", name: "api" }), repository({ id: "r2", name: "web" })],
      [member("a1", "Ada", "r1"), member("a2", "Grace", "r2")],
    );
    expect(screen.getAllByText("Ada")).toHaveLength(1);
    expect(screen.getAllByText("Grace")).toHaveLength(1);
  });

  it("leaves an agent in no repository to the crew below", () => {
    // The roster under the block draws those. Drawing them here as well is the
    // duplication the tree exists to remove.
    draw([repository()], [member("a1", "Ada"), member("a2", "Grace", "r1")]);
    expect(screen.queryByText("Ada")).toBeNull();
    expect(screen.getByText("Grace")).toBeTruthy();
  });

  it("says a repository is empty rather than drawing a gap", () => {
    draw([repository()], [member("a1", "Ada")]);
    expect(screen.getByText("nobody works here yet")).toBeTruthy();
  });

  it("turns the empty line into the instruction while something is dragging", () => {
    draw([repository()], [member("a1", "Ada")], true);
    expect(screen.getByText("drop an agent here")).toBeTruthy();
    expect(screen.queryByText("nobody works here yet")).toBeNull();
  });

  it("offers the repository as its own drop target", () => {
    // Dropping on a crew circle moves an agent between crews; dropping here
    // moves it between codebases inside the crew it is already in. Both moves,
    // different things moved, so they must never report the same target.
    const { onDragOver } = draw([repository()], [member("a1", "Ada")]);
    screen
      .getByText("guaca")
      .closest(".rail__repo")
      ?.dispatchEvent(new PointerEvent("pointerover", { bubbles: true }));
    expect(onDragOver).toHaveBeenCalledWith({ kind: "repository", id: "r1" });
  });

  it("carries the path where it can be read without crowding the name", () => {
    draw([repository()], [member("a1", "Ada")]);
    expect(screen.getByText("guaca").closest(".rail__repo-head")?.getAttribute("title")).toBe(
      "/Users/you/dev/guaca",
    );
  });
});
