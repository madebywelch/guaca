import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { AgentCard, RepoStatus, Repository } from "../lib/types";
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
    browserConsent: "open",
    repositoryId,
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
  };
}

function repository(over: Partial<Repository> = {}): Repository {
  return {
    id: "r1",
    groupId: GROUP,
    name: "guaca",
    path: "/Users/you/dev/guaca",
    note: "",
    harness: "pi",
    gate: "open",
    bench: "own",
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

/** The rail's own row, standing in for it. */
const row = (agent: AgentCard) => <div key={agent.id}>{agent.name}</div>;

function state(over: Partial<RepoStatus> = {}): RepoStatus {
  return {
    branch: "main",
    detached: false,
    dirty: 0,
    ahead: 0,
    behind: 0,
    upstream: true,
    pullRequests: null,
    ...over,
  };
}

function draw(
  repositories: Repository[],
  crew: AgentCard[],
  dragging = false,
  status: Record<string, RepoStatus> = {},
  building: Record<string, string> = {},
) {
  const onDragOver = vi.fn();
  const rendered = render(
    <RailRepositories
      repositories={repositories}
      crew={crew}
      status={status}
      building={building}
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

  it("says nothing about a repository it has not read yet", () => {
    // A row saying "main, clean" before anything had been asked is a claim
    // about a directory nobody looked at, and it reads exactly like an answer.
    const { container } = draw([repository()], [member("a1", "Ada")]);
    expect(container.querySelector(".rail__repo-state")).toBeNull();
  });

  it("draws the branch, what is uncommitted, and what is unpushed", () => {
    draw([repository()], [member("a1", "Ada")], false, {
      r1: state({ branch: "madebywelch/pi", dirty: 3, ahead: 2 }),
    });
    expect(screen.getByText("madebywelch/pi")).toBeTruthy();
    expect(screen.getByTitle("3 uncommitted")).toBeTruthy();
    expect(screen.getByTitle("2 to push")).toBeTruthy();
  });

  it("draws no arrows for a branch that tracks nothing", () => {
    // Without an upstream both counts are zero, which is not the same as being
    // in sync, so the arrows come off `upstream` rather than off the counts.
    draw([repository()], [member("a1", "Ada")], false, {
      r1: state({ upstream: false, ahead: 0, behind: 0 }),
    });
    expect(screen.queryByTitle(/to push/)).toBeNull();
    expect(screen.queryByTitle(/to pull/)).toBeNull();
  });

  it("says nothing about pull requests when gh could not be asked", () => {
    // null is not zero. gh missing, signed out, or a repository GitHub has
    // never heard of would all read as "nothing waiting for review".
    draw([repository()], [member("a1", "Ada")], false, {
      r1: state({ pullRequests: null }),
    });
    expect(screen.queryByText(/PR/)).toBeNull();
  });

  it("draws the count when there are open pull requests", () => {
    draw([repository()], [member("a1", "Ada")], false, {
      r1: state({ pullRequests: 4 }),
    });
    expect(screen.getByText("4 PR")).toBeTruthy();
    expect(screen.getByTitle("4 open pull requests")).toBeTruthy();
  });

  it("says a repository is building while a coding job runs in it", () => {
    // The job outlives the turn that started it, so the agent that asked has
    // already gone idle. Without this the crew reads as stopped at exactly the
    // moment it is building, which is what the operator asked about.
    draw([repository()], [member("a1", "Ada", "r1")], false, {}, { a1: "r1" });
    expect(screen.getByText("building")).toBeTruthy();
    expect(screen.getByTitle("a coding agent is working here")).toBeTruthy();
  });

  it("goes back to the member count when the job ends", () => {
    draw([repository()], [member("a1", "Ada", "r1")], false, {}, {});
    expect(screen.queryByText("building")).toBeNull();
  });

  it("marks a detached head as the state it is", () => {
    draw([repository()], [member("a1", "Ada")], false, {
      r1: state({ branch: "a1b2c3d", detached: true }),
    });
    expect(screen.getByTitle("detached HEAD")).toBeTruthy();
  });
});
