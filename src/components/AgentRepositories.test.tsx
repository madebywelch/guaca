import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, Repository } from "../lib/types";
import { AgentRepositories } from "./AgentRepositories";

const groupRepositories = vi.fn<(groupId: string) => Promise<Repository[]>>();
const setRepositoryAccess = vi.fn();

vi.mock("../lib/ipc", () => ({
  api: {
    groupRepositories: (groupId: string) => groupRepositories(groupId),
    setRepositoryAccess: (id: string, agentId: string, allowed: boolean) =>
      setRepositoryAccess(id, agentId, allowed),
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
  lifecycle: "active",
  pinned: false,
  railOrder: 0,
  version: 1,
  createdAt: 0,
  updatedAt: 0,
};

function repository(over: Partial<Repository> = {}): Repository {
  return {
    id: "r1",
    groupId: GROUP,
    name: "api",
    path: "/Users/you/dev/api",
    note: "",
    reach: [],
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

describe("AgentRepositories", () => {
  beforeEach(() => {
    groupRepositories.mockReset();
    setRepositoryAccess.mockReset();
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

  it("holds more than one, because a change often spans two", async () => {
    // Not one agent per repository. An agent that has the API and not the web
    // app has to hand half of an ordinary change to a peer and wait for it.
    groupRepositories.mockResolvedValue([
      repository({ id: "r1", name: "api", reach: ["a1"] }),
      repository({ id: "r2", name: "web", reach: ["a1"] }),
      repository({ id: "r3", name: "infra" }),
    ]);
    render(<AgentRepositories agent={ADA} />);

    expect(await screen.findByText(/Can write code in api, web, and nowhere else\./)).toBeTruthy();
    expect(screen.getByText("infra").getAttribute("aria-pressed")).toBe("false");
  });

  it("gives one to this agent and names only this agent", async () => {
    groupRepositories.mockResolvedValue([repository()]);
    setRepositoryAccess.mockResolvedValue(repository({ reach: ["a1"] }));
    render(<AgentRepositories agent={ADA} />);

    fireEvent.click(await screen.findByText("api"));

    await waitFor(() => expect(setRepositoryAccess).toHaveBeenCalledWith("r1", "a1", true));
  });

  it("takes one back", async () => {
    groupRepositories.mockResolvedValue([repository({ reach: ["a1"] })]);
    setRepositoryAccess.mockResolvedValue(repository());
    render(<AgentRepositories agent={ADA} />);

    const api = await screen.findByText("api");
    expect(api.getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(api);

    await waitFor(() => expect(setRepositoryAccess).toHaveBeenCalledWith("r1", "a1", false));
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
