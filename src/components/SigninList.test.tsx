import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, Signin } from "../lib/types";
import { SigninList } from "./SigninList";

const agentSignins = vi.fn<(id: string) => Promise<Signin[]>>();
const scanAgentSignins = vi.fn<(id: string) => Promise<Signin[]>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentSignins: (id: string) => agentSignins(id),
    scanAgentSignins: (id: string) => scanAgentSignins(id),
  },
}));

function card(over: Partial<AgentCard> = {}): AgentCard {
  return {
    id: "a1",
    groupId: "g1",
    sandboxId: "sb-1",
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    repositoryId: null,
    name: "Researcher",
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    discardedAt: null,
    ...over,
  };
}

function signin(over: Partial<Signin> = {}): Signin {
  return {
    agentId: "a1",
    surface: "browser",
    domain: "linkedin.com",
    service: "LinkedIn",
    recognized: true,
    firstSeenAt: Date.now() - 86_400_000,
    lastSeenAt: Date.now() - 60_000,
    ...over,
  };
}

describe("SigninList", () => {
  beforeEach(() => {
    agentSignins.mockReset();
    scanAgentSignins.mockReset();
    agentSignins.mockResolvedValue([]);
    scanAgentSignins.mockResolvedValue([]);
  });

  it("shows what the browser turned out to be signed in to", async () => {
    agentSignins.mockResolvedValue([signin()]);
    scanAgentSignins.mockResolvedValue([signin()]);
    render(<SigninList agent={card()} />);

    expect(await screen.findByText("LinkedIn")).toBeTruthy();
    expect(screen.getByText(/seen .* ago/)).toBeTruthy();
  });

  it("asks the machine on open rather than trusting the stored answer", async () => {
    // Sessions change on the machine, so a panel that only ever read the cache
    // would show a login the operator ended ten minutes ago.
    render(<SigninList agent={card()} />);
    await waitFor(() => expect(scanAgentSignins).toHaveBeenCalledWith("a1"));
  });

  it("offers nothing to fill in, because nothing is declared here", async () => {
    agentSignins.mockResolvedValue([signin()]);
    scanAgentSignins.mockResolvedValue([signin()]);
    const { container } = render(<SigninList agent={card()} />);
    await screen.findByText("LinkedIn");

    expect(container.querySelectorAll("input").length).toBe(0);
    expect(screen.queryByText("Add")).toBeNull();
  });

  it("marks a guess as a guess", async () => {
    // The weaker rule matches a session-shaped cookie on a visited site. It is
    // usually right and must not be presented as though it were certain.
    agentSignins.mockResolvedValue([
      signin({ service: "internal.example", domain: "internal.example", recognized: false }),
    ]);
    scanAgentSignins.mockResolvedValue([
      signin({ service: "internal.example", domain: "internal.example", recognized: false }),
    ]);
    render(<SigninList agent={card()} />);

    expect(await screen.findByText("looks signed in")).toBeTruthy();
  });

  it("says there is nothing to be signed in to when the agent has neither place", async () => {
    render(<SigninList agent={card({ sandboxId: null, browserId: null })} />);
    expect(await screen.findByText(/no computer and no browser yet/)).toBeTruthy();
    expect(scanAgentSignins).not.toHaveBeenCalled();
  });

  it("scans an agent that has only a browser", async () => {
    // Either place can hold a session, so an agent that has only ever used the
    // web is still worth asking. Keying this off the machine alone left every
    // browser-only agent reporting nothing for ever.
    render(<SigninList agent={card({ sandboxId: null, browserId: "kb-1" })} />);
    await screen.findByText(/Signed in/);
    expect(scanAgentSignins).toHaveBeenCalled();
  });

  it("explains that signing in is all there is to do", async () => {
    render(<SigninList agent={card()} />);
    expect(await screen.findByText(/sign in to a site there/)).toBeTruthy();
    expect(screen.getByText(/do not have to tell/)).toBeTruthy();
  });

  it("says which of the two places holds a session", async () => {
    // A computer and a browser have unrelated cookie jars. An operator looking
    // at "LinkedIn" needs to know which window the agent will find it in, and
    // an agent told only the service name reaches for the wrong tool.
    const both = [
      signin({ service: "LinkedIn", surface: "browser" }),
      signin({ service: "Gmail", domain: "google.com", surface: "computer" }),
    ];
    agentSignins.mockResolvedValue(both);
    scanAgentSignins.mockResolvedValue(both);
    render(<SigninList agent={card()} />);

    expect(await screen.findByText("in its browser")).toBeTruthy();
    expect(screen.getByText("on its screen")).toBeTruthy();
  });

  it("rechecks on demand", async () => {
    agentSignins.mockResolvedValue([]);
    render(<SigninList agent={card()} />);
    await waitFor(() => expect(scanAgentSignins).toHaveBeenCalledTimes(1));

    scanAgentSignins.mockResolvedValue([signin()]);
    fireEvent.click(screen.getByText("Check now"));
    expect(await screen.findByText("LinkedIn")).toBeTruthy();
  });
});
