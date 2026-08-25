import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import type { AgentCard, Browser, Computer } from "../lib/types";
import { Inspector } from "./Inspector";

const agentComputer = vi.fn<(id: string) => Promise<Computer | null>>();
const agentBrowser = vi.fn<(id: string) => Promise<Browser | null>>();
const agentRoutines = vi.fn<() => Promise<never[]>>();
const agentMemory = vi.fn<(id: string) => Promise<string>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentComputer: (id: string) => agentComputer(id),
    agentBrowser: (id: string) => agentBrowser(id),
    agentRoutines: () => agentRoutines(),
    agentMemory: (id: string) => agentMemory(id),
    setAgentMemory: vi.fn(),
    startAgentComputer: vi.fn(),
    stopAgentComputer: vi.fn(),
    deleteAgentComputer: vi.fn(),
    startAgentBrowser: vi.fn(),
    stopAgentBrowser: vi.fn(),
  },
}));

function card(id: string, name: string): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
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
  };
}

function configure() {
  useStore.setState({
    openingRoutine: null,
    routineVersion: {},
    memoryVersion: {},
    settings: {
      operatorName: "",
      e2bKeySet: true,
      e2bKeyHint: "",
      computerIdleMinutes: 15,
      kernelKeySet: true,
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
}

describe("Inspector", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    agentComputer.mockResolvedValue(null);
    agentBrowser.mockResolvedValue(null);
    agentRoutines.mockResolvedValue([]);
    agentMemory.mockImplementation(async (id) => `${id} remembers this`);
    localStorage.clear();
    configure();
  });

  it("holds one panel's worth of an agent, however many agents have been through it", async () => {
    // Two siblings under the same key left the first agent's screen in the DOM
    // when the second arrived: React's reconciler indexes the old children by
    // key, so the duplicate overwrote the entry it would have deleted through.
    // Every switch added another dead card, and only closing the panel, which
    // unmounts their container, cleared them.
    const view = render(<Inspector agent={card("a1", "Cook")} onEditProfile={vi.fn()} />);
    await screen.findByText("Cook's screen");

    for (const [id, name] of [
      ["a2", "Scribe"],
      ["a3", "Runner"],
    ] as const) {
      view.rerender(<Inspector agent={card(id, name)} onEditProfile={vi.fn()} />);
      await screen.findByText(`${name}'s screen`);
    }

    await waitFor(() => expect(screen.getAllByText(/'s screen$/)).toHaveLength(1));
    expect(screen.getAllByText(/'s browser$/)).toHaveLength(1);
    expect(screen.getAllByRole("heading", { name: "Routines" })).toHaveLength(1);
    expect(screen.getAllByLabelText("Memory")).toHaveLength(1);
    expect(screen.queryByText("Cook's screen")).toBeNull();
    expect(screen.queryByText("Scribe's screen")).toBeNull();
  });

  it("switches the memory over with everything else, rather than under the new name", async () => {
    // One key on the level they share is the whole mechanism, and a memory
    // left behind is the worst of the three to leave: an operator editing what
    // they believe is one agent's memory would save it onto another's.
    const view = render(<Inspector agent={card("a1", "Cook")} onEditProfile={vi.fn()} />);
    expect(await screen.findByDisplayValue("a1 remembers this")).toBeTruthy();

    view.rerender(<Inspector agent={card("a2", "Scribe")} onEditProfile={vi.fn()} />);

    expect(await screen.findByDisplayValue("a2 remembers this")).toBeTruthy();
    expect(screen.queryByDisplayValue("a1 remembers this")).toBeNull();
  });
});
