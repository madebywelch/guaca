import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, Computer } from "../lib/types";
import { ComputerPane } from "./ComputerPane";

const agentComputer = vi.fn<(id: string) => Promise<Computer | null>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentComputer: (id: string) => agentComputer(id),
    startAgentComputer: vi.fn(),
    stopAgentComputer: vi.fn(),
    deleteAgentComputer: vi.fn(),
  },
}));

function card(id: string, name: string): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    version: 1,
    createdAt: 0,
    updatedAt: 0,
  };
}

const HAS_ONE: Computer = {
  sandboxId: "sb-live",
  state: "running",
  vncUrl: "http://127.0.0.1:9/vnc.html",
};

describe("ComputerPane", () => {
  beforeEach(() => {
    agentComputer.mockReset();
    useStore.setState({
      settings: {
        operatorName: "",
        e2bKeySet: true,
        e2bKeyHint: "",
        computerIdleMinutes: 15,
        baseUrl: "",
        defaultModel: "",
        apiKeySet: true,
        apiKeyHint: "",
        requestTimeoutSecs: 120,
        limits: {
          maxHops: 8,
          maxStepsPerRun: 60,
          maxFanoutPerCall: 8,
          maxSendsPerPair: 6,
          maxToolRounds: 24,
        },
      },
    });
  });

  it("never shows one agent's machine in another agent's pane", async () => {
    // The lookup for the agent being left behind is deliberately the slower of
    // the two: it lands after the operator has already switched, which is the
    // whole bug. It used to paint its screen into the new agent's pane.
    let settleWithMachine: (value: Computer | null) => void = () => {};
    agentComputer.mockImplementation(
      (id) =>
        new Promise((resolve) => {
          if (id === "has-one") settleWithMachine = resolve;
          else resolve(null);
        }),
    );

    const view = render(<ComputerPane agent={card("has-one", "Cook")} />);
    view.rerender(<ComputerPane agent={card("has-none", "Scribe")} />);
    settleWithMachine(HAS_ONE);

    await waitFor(() => expect(screen.getByText(/No computer yet/)).toBeTruthy());
    expect(screen.queryByTitle(/computer$/)).toBeNull();
  });

  it("shows the machine of the agent actually being looked at", async () => {
    agentComputer.mockResolvedValue(HAS_ONE);
    render(<ComputerPane agent={card("has-one", "Cook")} />);
    await waitFor(() => expect(screen.getByTitle("Cook's computer")).toBeTruthy());
  });
});
