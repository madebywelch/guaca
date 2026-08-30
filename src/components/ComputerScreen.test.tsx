import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, Computer } from "../lib/types";
import { ComputerScreen } from "./ComputerScreen";

const agentComputer = vi.fn<(id: string) => Promise<Computer | null>>();
const giveAgentComputer = vi.fn<(id: string) => Promise<unknown>>();
const takeAgentComputer = vi.fn<(id: string) => Promise<unknown>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentComputer: (id: string) => agentComputer(id),
    giveAgentComputer: (id: string) => giveAgentComputer(id),
    takeAgentComputer: (id: string) => takeAgentComputer(id),
    startAgentComputer: vi.fn(),
    stopAgentComputer: vi.fn(),
    deleteAgentComputer: vi.fn(),
  },
}));

function card(id: string, name: string, given = true): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: given,
    hasBrowser: false,
    browserConsent: "open",
    repositoryId: null,
    name,
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
    giveAgentComputer.mockReset();
    giveAgentComputer.mockResolvedValue(undefined);
    takeAgentComputer.mockReset();
    takeAgentComputer.mockResolvedValue(undefined);
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
        provider: "compatible",
        subscriptionModel: "gpt-5.6-luna",
        subscriptionModels: ["gpt-5.6-luna", "gpt-5.4-mini"],
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
    expect(screen.getByText(/Scribe has a computer and no machine yet/)).toBeTruthy();
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

  it("holds the space the screen had while it covers the window", async () => {
    // The picture leaves the flow to grow, so something has to stay behind
    // holding its place. Without it the rest of the panel jumps up the moment
    // the operator asks for a better look, and back down when they close it.
    agentComputer.mockResolvedValue(HAS_ONE);
    const { container } = render(<ComputerScreen agent={card("has-one", "Cook")} />);
    fireEvent.click(await screen.findByRole("button", { name: /take over/i }));

    const held = container.querySelector(".screen");
    const stage = screen.getByRole("dialog", { name: "Cook's computer" });
    expect(held).not.toBe(stage);
    expect(held?.contains(stage)).toBe(true);
    expect(held?.getAttribute("data-full")).toBe("true");
  });

  it("grows out of the picture it came from rather than appearing at full size", async () => {
    // FLIP: the stage is already covering the window by the time this runs, so
    // it is put back over the small picture and let go. A change of size that
    // lands in a single frame reads as a reconnect that never happened.
    agentComputer.mockResolvedValue(HAS_ONE);
    render(<ComputerScreen agent={card("has-one", "Cook")} />);
    const veil = await screen.findByRole("button", { name: /take over/i });

    // jsdom does no layout, so the two measurements the movement is made of are
    // supplied: the small picture, then the window the stage grew into.
    const small = { top: 40, left: 100, width: 300, height: 188 } as DOMRect;
    const whole = { top: 0, left: 0, width: 1200, height: 800 } as DOMRect;
    const measure = vi
      .spyOn(HTMLElement.prototype, "getBoundingClientRect")
      .mockReturnValueOnce(small)
      .mockReturnValue(whole);
    fireEvent.click(veil);
    measure.mockRestore();

    const stage = screen.getByRole("dialog", { name: "Cook's computer" });
    expect(stage.style.transform).toBe("translate(100px, 40px) scale(0.25, 0.235)");

    // And then let go, which is what actually plays the movement.
    await waitFor(() => expect(stage.style.transform).toBe(""));
    expect(stage.dataset.zooming).toBe("true");
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

  it("offers a computer to an agent that has not been given one, and asks nobody", async () => {
    // The decision is the operator's and it is made here. Asking the provider
    // what an agent that may not have a machine is doing is a round trip for an
    // answer that cannot change what the panel draws.
    render(<ComputerScreen agent={card("has-none", "Scribe", false)} />);

    await waitFor(() => expect(screen.getByText(/Scribe has no computer/)).toBeTruthy());
    expect(agentComputer).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Take it back" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Give one" }));
    await waitFor(() => expect(giveAgentComputer).toHaveBeenCalledWith("has-none"));
    // And nothing is rented by saying yes: a machine is still made the first
    // time one is needed.
    expect(screen.queryByTitle("Scribe's computer")).toBeNull();
  });

  it("takes it back without waiting for the operator to open the screen", async () => {
    // A live picture fills the panel, so the control that ends the arrangement
    // has to be reachable from the empty state and from the bar above the
    // picture. This is the empty one: given, with no machine yet.
    agentComputer.mockResolvedValue(null);
    render(<ComputerScreen agent={card("has-one", "Cook")} />);

    await waitFor(() => expect(screen.getByText(/Cook has a computer/)).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "Take it back" }));
    await waitFor(() => expect(takeAgentComputer).toHaveBeenCalledWith("has-one"));
  });

  it("keeps the picture off the panel of an agent whose computer was taken back", async () => {
    // The machine outlives the decision on purpose: its disk holds the
    // operator's sign-ins. What must not outlive it is the screen, which is a
    // live connection to a machine this agent may no longer use.
    agentComputer.mockResolvedValue(HAS_ONE);
    const view = render(<ComputerScreen agent={card("has-one", "Cook")} />);
    await waitFor(() => expect(screen.getByTitle("Cook's computer")).toBeTruthy());

    view.rerender(<ComputerScreen agent={card("has-one", "Cook", false)} />);
    await waitFor(() => expect(screen.queryByTitle("Cook's computer")).toBeNull());
    expect(screen.getByText(/Cook has no computer/)).toBeTruthy();
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
