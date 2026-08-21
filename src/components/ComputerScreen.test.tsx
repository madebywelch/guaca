import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, Computer } from "../lib/types";
import { ComputerScreen } from "./ComputerScreen";

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
    browserId: null,
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
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

describe("ComputerScreen", () => {
  beforeEach(() => {
    agentComputer.mockReset();
    useStore.setState({
      settings: {
        operatorName: "",
        e2bKeySet: true,
        e2bKeyHint: "",
        computerIdleMinutes: 15,
        kernelKeySet: false,
        kernelKeyHint: "",
        browserIdleMinutes: 60,
        browserStealth: false,
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

  it("never shows one agent's machine in another agent's panel", async () => {
    // The lookup for the agent being left behind is deliberately the slower of
    // the two: it lands after the operator has already switched, which is the
    // whole bug. It used to paint its screen into the new agent's panel.
    let settleWithMachine: (value: Computer | null) => void = () => {};
    agentComputer.mockImplementation(
      (id) =>
        new Promise((resolve) => {
          if (id === "has-one") settleWithMachine = resolve;
          else resolve(null);
        }),
    );

    const view = render(<ComputerScreen agent={card("has-one", "Cook")} />);
    view.rerender(<ComputerScreen agent={card("has-none", "Scribe")} />);
    settleWithMachine(HAS_ONE);

    await waitFor(() => expect(screen.getByText("Scribe's screen")).toBeTruthy());
    expect(screen.queryByTitle(/'s computer$/)).toBeNull();
    expect(screen.getByText(/No computer yet/)).toBeTruthy();
  });

  it("shows the machine of the agent actually being looked at", async () => {
    agentComputer.mockResolvedValue(HAS_ONE);
    render(<ComputerScreen agent={card("has-one", "Cook")} />);
    await waitFor(() => expect(screen.getByTitle("Cook's computer")).toBeTruthy());
    expect(screen.getByText("Cook's screen")).toBeTruthy();
  });

  it("keeps a click off the desktop until the operator asks for it", async () => {
    // A pointer landing in an agent's desktop by accident is worse than one
    // extra click to take control, so the preview is watched through a veil.
    agentComputer.mockResolvedValue(HAS_ONE);
    render(<ComputerScreen agent={card("has-one", "Cook")} />);

    const veil = await screen.findByRole("button", { name: /take over/i });
    fireEvent.click(veil);

    // Full screen: the veil is gone and the controls that touch the machine
    // are reachable.
    expect(screen.queryByRole("button", { name: /take over/i })).toBeNull();
    expect(screen.getByRole("button", { name: "Sleep" })).toBeTruthy();
    expect(screen.getByRole("dialog", { name: "Cook's computer" })).toBeTruthy();
  });

  it("keeps the same connection open across the change of size", async () => {
    // Remounting the frame would drop the desktop and reconnect to it, which
    // is a visible stall every time the operator wants a better look.
    agentComputer.mockResolvedValue(HAS_ONE);
    render(<ComputerScreen agent={card("has-one", "Cook")} />);

    const before = await screen.findByTitle("Cook's computer");
    fireEvent.click(screen.getByRole("button", { name: /take over/i }));
    expect(screen.getByTitle("Cook's computer")).toBe(before);
  });

  it("shrinks again on escape, without needing somewhere else to click first", async () => {
    // The desktop swallows key presses once it has focus, so this is listened
    // for on the window rather than on the frame.
    agentComputer.mockResolvedValue(HAS_ONE);
    render(<ComputerScreen agent={card("has-one", "Cook")} />);

    fireEvent.click(await screen.findByRole("button", { name: /take over/i }));
    fireEvent.keyDown(window, { key: "Escape" });
    await waitFor(() => expect(screen.getByRole("button", { name: /take over/i })).toBeTruthy());
  });

  it("says nothing at all when computers were never set up", async () => {
    // Offering to give an agent a machine that cannot be made is worse than
    // not mentioning computers.
    useStore.setState({ settings: { ...useStore.getState().settings!, e2bKeySet: false } });
    const { container } = render(<ComputerScreen agent={card("has-none", "Scribe")} />);
    await waitFor(() => expect(container.firstChild).toBeNull());
    expect(agentComputer).not.toHaveBeenCalled();
  });
});
