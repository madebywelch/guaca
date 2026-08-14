import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, Settings } from "./lib/types";

/**
 * Smoke test for the shell.
 *
 * A blank window is the worst failure this app can have: nothing throws, the
 * background paints, and there is no clue what went wrong. This mounts the real
 * component tree over a mocked IPC layer, so anything that empties the window
 * fails here instead of in a release bundle.
 */

const listAgents = vi.fn<() => Promise<AgentCard[]>>(async () => []);
const agentLastActive = vi.fn<() => Promise<Record<string, number>>>(async () => ({}));
const getSettings = vi.fn<() => Promise<Settings>>(async () => ({
  baseUrl: "https://openrouter.ai/api/v1",
  defaultModel: "test/model",
  apiKeySet: true,
  apiKeyHint: "...9999",
  requestTimeoutSecs: 120,
  limits: { maxHops: 4, maxStepsPerRun: 40, maxFanoutPerCall: 8, maxSendsPerPair: 3 },
}));

vi.mock("./lib/ipc", () => ({
  api: {
    listAgents: () => listAgents(),
    listGroups: async () => [
      {
        id: "00000000-0000-4000-8000-000000000001",
        name: "Everyone",
        agentCount: 3,
        createdAt: 0,
        baseUrl: null,
        defaultModel: null,
        apiKeySet: false,
        apiKeyHint: "",
      },
    ],
    agentActivity: async () => ({}),
    agentLastActive: () => agentLastActive(),
    getSettings: () => getSettings(),
    channelMessages: async () => [],
    activityFeed: async () => [],
    createAgent: async () => {
      throw new Error("not used");
    },
  },
  onRuntimeEvent: async () => () => {},
}));

const { default: App } = await import("./App");
const { useStore } = await import("./lib/store");

function agent(name: string): AgentCard {
  return {
    id: `id-${name}`,
    groupId: "00000000-0000-4000-8000-000000000001",
    name,
    avatar: "avocado",
    color: "#c7d96b",
    model: "test/model",
    systemPrompt: "",
    skills: ["testing"],
    lifecycle: "active",
    version: 1,
    createdAt: 1,
    updatedAt: 1,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  listAgents.mockResolvedValue([]);
  agentLastActive.mockResolvedValue({});
  useStore.setState({
    agents: [],
    activity: {},
    lastActive: {},
    settings: null,
    selected: null,
    messages: {},
    streams: {},
    pulses: [],
    banner: null,
  });
});

describe("App", () => {
  it("renders the rail", async () => {
    render(<App />);
    // The rail is always present, whether or not there are agents. If this is
    // missing, the window is blank.
    expect(await screen.findByRole("navigation", { name: /agents/i })).toBeTruthy();
    expect(screen.getByText("Guac")).toBeTruthy();
  });

  it("offers a way in when there are no agents", async () => {
    render(<App />);
    expect(await screen.findByRole("button", { name: /starter crew/i })).toBeTruthy();
    expect(screen.getByRole("button", { name: /create one agent/i })).toBeTruthy();
  });

  it("lists agents and opens a channel", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef")]);
    render(<App />);

    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    expect(screen.getByText("Chef")).toBeTruthy();
    // A channel header, meaning the pane rendered rather than staying empty.
    await waitFor(() => expect(screen.getByRole("heading", { name: "Manager" })).toBeTruthy());
  });

  it("puts the most recently active agent at the top of the rail", async () => {
    listAgents.mockResolvedValue([agent("Manager"), agent("Chef"), agent("Host")]);
    agentLastActive.mockResolvedValue({ "id-Host": 900, "id-Chef": 500 });
    render(<App />);

    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    const rail = screen.getByRole("navigation", { name: /agents/i });
    const names = [...rail.querySelectorAll(".agent-row__name")].map((n) => n.textContent);
    // Host spoke last, then Chef. Manager has never spoken, so it keeps its
    // creation position at the bottom.
    expect(names).toEqual(["Host", "Chef", "Manager"]);
  });

  it("keeps deleted agents out of the rail", async () => {
    listAgents.mockResolvedValue([
      agent("Manager"),
      { ...agent("Ghost"), lifecycle: "terminated" },
    ]);
    render(<App />);
    await waitFor(() => expect(screen.getByText("Manager")).toBeTruthy());
    expect(screen.queryByText("Ghost")).toBeNull();
  });

  it("prompts for a key when none is configured", async () => {
    getSettings.mockResolvedValue({
      baseUrl: "https://openrouter.ai/api/v1",
      defaultModel: "test/model",
      apiKeySet: false,
      apiKeyHint: "",
      requestTimeoutSecs: 120,
      limits: { maxHops: 4, maxStepsPerRun: 40, maxFanoutPerCall: 8, maxSendsPerPair: 3 },
    });
    render(<App />);
    expect(await screen.findByText(/Add an API key/i)).toBeTruthy();
  });

  it("still renders the rail when startup fails", async () => {
    // A failed bootstrap must degrade to a usable window with an error, not to
    // a blank one.
    listAgents.mockRejectedValue({ kind: "storage", message: "disk on fire" });
    render(<App />);
    expect(await screen.findByRole("navigation", { name: /agents/i })).toBeTruthy();
    expect(await screen.findByText(/disk on fire/i)).toBeTruthy();
  });
});

describe("failure surfacing", () => {
  it("shows the error instead of a blank window when a child throws", async () => {
    const { ErrorBoundary } = await import("./components/ErrorBoundary");
    const Boom = (): never => {
      throw new Error("render exploded");
    };
    // React logs the caught error; silence it so the run stays readable.
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});
    render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>,
    );
    expect(screen.getByText(/could not draw this window/i)).toBeTruthy();
    expect(screen.getByText(/render exploded/)).toBeTruthy();
    spy.mockRestore();
  });
});
