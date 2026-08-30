import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, Repository } from "../lib/types";
import { AgentRepositories } from "./AgentRepositories";

const groupRepositories = vi.fn<(groupId: string) => Promise<Repository[]>>();
const setAgentRepository = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: {
    groupRepositories: (groupId: string) => groupRepositories(groupId),
    setAgentRepository: (id: string, repositoryId: string | null) =>
      setAgentRepository(id, repositoryId),
  },
}));

const GROUP = "00000000-0000-4000-8000-000000000001";

const ADA: AgentCard = {
  id: "a1",
  groupId: GROUP,
  name: "Ada",
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
  repositoryId: null,
  lifecycle: "active",
  pinned: false,
  railOrder: 0,
  version: 1,
  createdAt: 0,
  updatedAt: 0,
  discardedAt: null,
};

function repository(over: Partial<Repository> = {}): Repository {
  return {
    id: "r1",
    groupId: GROUP,
    name: "api",
    path: "/Users/you/dev/api",
    note: "",
    harness: "pi",
    gate: "open",
    bench: "own",
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

describe("AgentRepositories", () => {
  beforeEach(() => {
    groupRepositories.mockReset();
    setAgentRepository.mockReset();
    groupRepositories.mockResolvedValue([]);
  });

  it("says an agent with nothing ticked cannot write code", async () => {
    // This panel is where an operator looks for the engineer switch, so it has
    // to answer the question the switch would have answered. Silence here reads
    // as an agent that can code and has not been pointed anywhere yet.
    groupRepositories.mockResolvedValue([repository()]);
    render(<AgentRepositories agent={ADA} />);

    expect(await screen.findByText(/cannot write code/)).toBeTruthy();
  });

  it("draws the one it is in as chosen and the rest as not", async () => {
    // One at a time. Two agents on one codebase coordinate in the crew they
    // share; one agent holding two is a change nobody can see the shape of.
    groupRepositories.mockResolvedValue([
      repository({ id: "r1", name: "api" }),
      repository({ id: "r2", name: "web" }),
    ]);
    render(<AgentRepositories agent={{ ...ADA, repositoryId: "r1" }} />);

    expect((await screen.findByText("api")).getAttribute("aria-pressed")).toBe("true");
    expect(screen.getByText("web").getAttribute("aria-pressed")).toBe("false");
    expect(screen.getByText(/Works in api, and nowhere else/)).toBeTruthy();
  });

  it("puts the agent in one", async () => {
    groupRepositories.mockResolvedValue([repository()]);
    setAgentRepository.mockResolvedValue(undefined);
    render(<AgentRepositories agent={ADA} />);

    fireEvent.click(await screen.findByText("api"));

    await waitFor(() => expect(setAgentRepository).toHaveBeenCalledWith("a1", "r1"));
  });

  it("moves rather than adding when a second is chosen", async () => {
    groupRepositories.mockResolvedValue([
      repository({ id: "r1", name: "api" }),
      repository({ id: "r2", name: "web" }),
    ]);
    setAgentRepository.mockResolvedValue(undefined);
    render(<AgentRepositories agent={{ ...ADA, repositoryId: "r1" }} />);

    fireEvent.click(await screen.findByText("web"));

    await waitFor(() => expect(setAgentRepository).toHaveBeenCalledWith("a1", "r2"));
  });

  it("takes the agent out when the one it is in is chosen again", async () => {
    // A set of buttons where one is always pressed has no other way back to
    // none, and none is a state the operator has to be able to reach.
    groupRepositories.mockResolvedValue([repository()]);
    setAgentRepository.mockResolvedValue(undefined);
    render(<AgentRepositories agent={{ ...ADA, repositoryId: "r1" }} />);

    fireEvent.click(await screen.findByText("api"));

    await waitFor(() => expect(setAgentRepository).toHaveBeenCalledWith("a1", null));
  });

  it("is drawn with nothing to offer, and says where repositories come from", async () => {
    // Unlike the standing grants beside it, which are hidden when empty. An
    // absent panel here reads as a feature that does not exist rather than as a
    // crew that has not linked anything.
    render(<AgentRepositories agent={ADA} />);

    expect(await screen.findByText(/no repositories linked/)).toBeTruthy();
    expect(screen.getByText(/group's settings/)).toBeTruthy();
  });

  it("carries the path where it can be read without crowding the name", async () => {
    // Two repositories called `api` in two crews is ordinary. The name is what
    // the operator ticks; the path is what tells them which one it is.
    groupRepositories.mockResolvedValue([repository()]);
    render(<AgentRepositories agent={ADA} />);

    expect((await screen.findByText("api")).getAttribute("title")).toBe("/Users/you/dev/api");
  });
});
