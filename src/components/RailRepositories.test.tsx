import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AgentCard, Repository } from "../lib/types";
import { RailRepositories } from "./RailRepositories";

const GROUP = "00000000-0000-4000-8000-000000000001";

function member(id: string, name: string, groupId = GROUP): AgentCard {
  return {
    id,
    groupId,
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
    reach: [],
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

const CREW = [member("a1", "Ada"), member("a2", "Grace")];

function draw(repositories: Repository[], crew = CREW, dragging = false) {
  const onDragOver = vi.fn();
  render(
    <RailRepositories
      repositories={repositories}
      crew={crew}
      isOver={() => false}
      onDragOver={onDragOver}
      onDragLeave={vi.fn()}
      dragging={dragging}
    />,
  );
  return { onDragOver };
}

describe("RailRepositories", () => {
  it("draws nothing when a crew has no repositories", () => {
    // Furniture that is always there is furniture, and a crew with no codebase
    // is most crews. The rail is an agent list first.
    const { container } = render(
      <RailRepositories
        repositories={[]}
        crew={CREW}
        isOver={() => false}
        onDragOver={vi.fn()}
        onDragLeave={vi.fn()}
        dragging={false}
      />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("names who is on each one", () => {
    draw([repository({ reach: ["a1"] })]);
    expect(screen.getByText("guaca")).toBeTruthy();
    expect(screen.getByText("Ada")).toBeTruthy();
  });

  it("says nobody rather than drawing an empty line", () => {
    // A repository linked and not handed out is a state to pass through, and
    // the row has to say which state it is in: blank reads as still loading.
    draw([repository()]);
    expect(screen.getByText("nobody yet")).toBeTruthy();
  });

  it("turns the empty line into the instruction while something is dragging", () => {
    draw([repository()], CREW, true);
    expect(screen.getByText("drop to give it this")).toBeTruthy();
    expect(screen.queryByText("nobody yet")).toBeNull();
  });

  it("never names an agent the runtime would refuse", () => {
    // Reach outlives a move between crews until something clears it. A
    // permission panel naming somebody who would be refused is the one thing it
    // must not do, so a name is drawn only if that agent is still in this crew.
    draw([repository({ reach: ["a1", "gone"] })]);
    expect(screen.getByText("Ada")).toBeTruthy();
    expect(screen.queryByText(/gone/)).toBeNull();
  });

  it("offers the repository as its own drop target, not the crew's", () => {
    // The distinction the whole block exists for. Dropping on a crew moves an
    // agent, because it is in exactly one; dropping here grants, because it can
    // work in several. If these ever report the same target, an operator loses
    // an agent while trying to give it a codebase.
    const { onDragOver } = draw([repository()]);
    screen
      .getByText("guaca")
      .closest(".rail__repo")
      ?.dispatchEvent(new PointerEvent("pointerover", { bubbles: true }));
    expect(onDragOver).toHaveBeenCalledWith({ kind: "repository", id: "r1" });
  });

  it("carries the path where it can be read without crowding the name", () => {
    draw([repository()]);
    expect(screen.getByText("guaca").closest(".rail__repo")?.getAttribute("title")).toBe(
      "/Users/you/dev/guaca",
    );
  });
});
