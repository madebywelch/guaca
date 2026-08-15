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
    name: "Researcher",
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    version: 1,
    createdAt: 0,
    updatedAt: 0,
    ...over,
  };
}

function signin(over: Partial<Signin> = {}): Signin {
  return {
    agentId: "a1",
    domain: "linkedin.com",
    service: "LinkedIn",
    recognised: true,
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
      signin({ service: "internal.example", domain: "internal.example", recognised: false }),
    ]);
    scanAgentSignins.mockResolvedValue([
      signin({ service: "internal.example", domain: "internal.example", recognised: false }),
    ]);
    render(<SigninList agent={card()} />);

    expect(await screen.findByText("looks signed in")).toBeTruthy();
  });

  it("says there is nothing to be signed in to when the agent has no computer", async () => {
    render(<SigninList agent={card({ sandboxId: null })} />);
    expect(await screen.findByText(/no computer yet/)).toBeTruthy();
    expect(scanAgentSignins).not.toHaveBeenCalled();
  });

  it("explains that signing in on the screen is all there is to do", async () => {
    render(<SigninList agent={card()} />);
    expect(await screen.findByText(/sign in to a site on its screen/)).toBeTruthy();
    expect(screen.getByText(/do not have to tell Researcher/)).toBeTruthy();
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
