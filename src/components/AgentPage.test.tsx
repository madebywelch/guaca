import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useStore } from "../lib/store";
import { whenLabel } from "../lib/time";
import type { AgentCard, Browser, Computer } from "../lib/types";
import { AgentPanel } from "./AgentPanel";
import { VIEWS } from "./ChannelView";

const agentComputer = vi.fn<(id: string) => Promise<Computer | null>>();
const agentBrowser = vi.fn<(id: string) => Promise<Browser | null>>();
const agentRoutines = vi.fn<() => Promise<never[]>>();
const agentMemory = vi.fn<(id: string) => Promise<string>>();
const retryTurn = vi.fn<(agent: string, message: string) => Promise<string>>();

vi.mock("../lib/ipc", () => ({
  api: {
    agentComputer: (id: string) => agentComputer(id),
    agentBrowser: (id: string) => agentBrowser(id),
    agentRoutines: () => agentRoutines(),
    agentMemory: (id: string) => agentMemory(id),
    agentMemoryVersioned: async (id: string) => ({
      content: await agentMemory(id),
      version: "version",
    }),
    agentWorkingNotes: async () => [],
    agentNotebook: async () => [],
    agentTerminal: async () => ({ path: "/var/lib/guaca/data/terminals/a1" }),
    codingHarnesses: async () => [],
    setAgentMemory: vi.fn(),
    startAgentComputer: vi.fn(),
    stopAgentComputer: vi.fn(),
    deleteAgentComputer: vi.fn(),
    startAgentBrowser: vi.fn(),
    stopAgentBrowser: vi.fn(),
    retryTurn: (agent: string, message: string) => retryTurn(agent, message),
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
    runsErrands: false,
    readsWeb: true,
    browserConsent: "open",
    name,
    avatar: "plain",
    color: "#c7d96b",
    model: "m",
    subscriptionModel: "",
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
      searchKeySet: false,
      searchKeyHint: "",
      searchesWith: "",
      baseUrl: "",
      defaultModel: "",
      provider: "compatible",
      subscriptionModel: "gpt-5.6-luna",
      reasoningEffort: "auto",
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

describe("the agent's page, past the conversation", () => {
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
    const view = render(
      <AgentPanel section="now" agent={card("a1", "Cook")} onEditProfile={vi.fn()} />,
    );
    await screen.findByRole("region", { name: "Cook's computer" });

    for (const [id, name] of [
      ["a2", "Scribe"],
      ["a3", "Runner"],
    ] as const) {
      view.rerender(<AgentPanel section="now" agent={card(id, name)} onEditProfile={vi.fn()} />);
      await screen.findByRole("region", { name: `${name}'s computer` });
    }

    await waitFor(() =>
      expect(screen.getAllByRole("region", { name: /'s computer$/ })).toHaveLength(1),
    );
    expect(screen.getAllByRole("region", { name: /'s browser$/ })).toHaveLength(1);
    expect(screen.queryByRole("region", { name: "Cook's computer" })).toBeNull();
    expect(screen.queryByRole("region", { name: "Scribe's computer" })).toBeNull();
    view.unmount();

    for (const [section, check] of [
      [
        "routines",
        () => expect(screen.getAllByRole("heading", { name: "Routines" })).toHaveLength(1),
      ],
      ["memory", () => expect(screen.getAllByRole("button", { name: "Edit" })).toHaveLength(1)],
    ] as const) {
      const tab = render(
        <AgentPanel section={section} agent={card("a1", "Cook")} onEditProfile={vi.fn()} />,
      );
      for (const [id, name] of [
        ["a2", "Scribe"],
        ["a3", "Runner"],
      ] as const) {
        tab.rerender(
          <AgentPanel section={section} agent={card(id, name)} onEditProfile={vi.fn()} />,
        );
      }
      await waitFor(check);
      tab.unmount();
    }
  });

  it("draws what the agent is doing on Now: the task, its steps, then the places", async () => {
    useStore.setState({
      agents: [card("a1", "Cook")],
      activity: { a1: { state: "thinking" } },
      activeRun: { a1: "r1" },
      trail: {
        a1: [
          {
            callId: "c1",
            name: "browse",
            arguments: { action: "open", url: "https://plumbers.example" },
            done: null,
            startedAt: 0,
          },
        ],
      },
      messages: {
        a1: [
          {
            id: "m1",
            runId: "r1",
            channelId: "a1",
            from: { kind: "human" },
            to: { kind: "agent", id: "a1" },
            parts: [{ type: "text", text: "Find a plumber for Thursday" }],
            trust: "operator",
            hop: 0,
            expectsReply: true,
            intent: "work",
            cause: null,
            createdAt: Date.now(),
          },
        ],
      },
    });
    render(<AgentPanel section="now" agent={card("a1", "Cook")} onEditProfile={vi.fn()} />);
    await screen.findByRole("heading", { name: "Computer" });

    const order = screen
      .getAllByRole("heading", { level: 3 })
      .map((heading) => heading.textContent);
    expect(order).toEqual(["Working on a request from you", "Steps", "Browser", "Computer"]);
    expect(screen.getByText("Find a plumber for Thursday")).toBeTruthy();
    expect(screen.getByRole("listitem").textContent).toBe("Opening plumbers.example");
    expect(screen.queryByRole("heading", { name: "Routines" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Memory" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Notebook" })).toBeNull();

    expect(VIEWS.map((tab) => tab.label)).toEqual([
      "Now",
      "Conversation",
      "Memory",
      "Notebook",
      "Routines",
      "Profile",
    ]);
  });

  it("offers Try again beside a failed last run, and sends the message that failed", async () => {
    retryTurn.mockResolvedValue("r2");
    useStore.setState({
      agents: [card("a1", "Cook")],
      activity: { a1: { state: "idle" } },
      activeRun: {},
      trail: {},
      messages: {
        a1: [
          {
            id: "m2",
            runId: "r1",
            channelId: "a1",
            trust: "system",
            hop: 0,
            expectsReply: false,
            intent: "courtesy",
            cause: "m1",
            from: { kind: "system" },
            to: { kind: "agent", id: "a1" },
            parts: [
              {
                type: "notice",
                kind: "upstreamError",
                text: "Cook could not reply: the provider had a problem on its side.",
              },
            ],
            createdAt: Date.now(),
          },
        ],
      },
    });
    render(<AgentPanel section="now" agent={card("a1", "Cook")} onEditProfile={vi.fn()} />);

    fireEvent.click(await screen.findByRole("button", { name: "Try again" }));
    expect(retryTurn).toHaveBeenCalledWith("a1", "m1");
    expect(await screen.findByRole("button", { name: "Sent again" })).toBeTruthy();
  });

  it("says the last run failed, and when, for an idle agent whose run failed", async () => {
    const at = Date.now() - 3_600_000;
    const base = {
      runId: "r1",
      channelId: "a1",
      trust: "system",
      hop: 0,
      expectsReply: false,
      intent: "work",
      cause: null,
    } as const;
    useStore.setState({
      agents: [card("a1", "Cook")],
      activity: { a1: { state: "idle" } },
      activeRun: {},
      trail: {},
      messages: {
        a1: [
          {
            ...base,
            id: "m1",
            from: { kind: "agent", id: "a1" },
            to: { kind: "human" },
            parts: [],
            createdAt: at - 1000,
          },
          {
            ...base,
            id: "m2",
            from: { kind: "system" },
            to: { kind: "agent", id: "a1" },
            parts: [
              {
                type: "notice",
                kind: "upstreamError",
                text: "Cook could not reply: could not reach the inference endpoint",
              },
            ],
            createdAt: at,
          },
        ],
      },
    });
    render(<AgentPanel section="now" agent={card("a1", "Cook")} onEditProfile={vi.fn()} />);

    const line = (await screen.findByRole("heading", { name: "Nothing is running" }))
      .nextElementSibling as HTMLElement;
    expect(line.textContent).toMatch(new RegExp(`^Last run failed ${whenLabel(at, Date.now())} `));
    expect(line.textContent).toContain("could not reach the inference endpoint");
    expect(line.textContent).not.toContain("Last finished");
  });

  it("says only that it is working when the run it is on is not known", async () => {
    useStore.setState({
      agents: [card("a1", "Cook")],
      activity: { a1: { state: "thinking" } },
      activeRun: {},
      trail: {},
      messages: {
        a1: [
          {
            id: "m1",
            runId: "r1",
            channelId: "a1",
            from: { kind: "human" },
            to: { kind: "agent", id: "a1" },
            parts: [{ type: "text", text: "Find a plumber for Thursday" }],
            trust: "operator",
            hop: 0,
            expectsReply: true,
            intent: "work",
            cause: null,
            createdAt: Date.now(),
          },
        ],
      },
    });
    render(<AgentPanel section="now" agent={card("a1", "Cook")} onEditProfile={vi.fn()} />);
    expect(await screen.findByRole("heading", { name: "Working" })).toBeTruthy();
    expect(screen.queryByText("Find a plumber for Thursday")).toBeNull();
  });

  it("names the request that started the run it is on, not whatever was said to it last", async () => {
    const said = (id: string, runId: string, text: string, createdAt: number) => ({
      id,
      runId,
      channelId: "a1",
      from: { kind: "human" as const },
      to: { kind: "agent" as const, id: "a1" },
      parts: [{ type: "text" as const, text }],
      trust: "operator" as const,
      hop: 0,
      expectsReply: true,
      intent: "work" as const,
      cause: null,
      createdAt,
    });
    useStore.setState({
      agents: [card("a1", "Cook")],
      activity: { a1: { state: "thinking" } },
      activeRun: { a1: "r1" },
      trail: {},
      messages: {
        a1: [
          said("m1", "r1", "Find a plumber for Thursday", Date.now() - 60_000),
          said("m2", "r2", "Also, the cat needs feeding", Date.now()),
        ],
      },
    });
    render(<AgentPanel section="now" agent={card("a1", "Cook")} onEditProfile={vi.fn()} />);
    expect(await screen.findByText("Find a plumber for Thursday")).toBeTruthy();
    expect(screen.queryByText("Also, the cat needs feeding")).toBeNull();
  });

  it("says plainly that it is waiting for an answer", async () => {
    useStore.setState({
      agents: [card("a1", "Cook")],
      activity: { a1: { state: "awaitingApproval" } },
      activeRun: {},
      trail: {},
      messages: { a1: [] },
    });
    render(<AgentPanel section="now" agent={card("a1", "Cook")} onEditProfile={vi.fn()} />);
    expect(
      await screen.findByRole("heading", { name: "Waiting for your answer before it goes on" }),
    ).toBeTruthy();
    expect(screen.queryByText(/with a request from you/)).toBeNull();
  });

  it("switches the memory over with everything else, rather than under the new name", async () => {
    const view = render(
      <AgentPanel section="memory" agent={card("a1", "Cook")} onEditProfile={vi.fn()} />,
    );
    expect(await screen.findByText("a1 remembers this")).toBeTruthy();

    view.rerender(
      <AgentPanel section="memory" agent={card("a2", "Scribe")} onEditProfile={vi.fn()} />,
    );

    expect(await screen.findByText("a2 remembers this")).toBeTruthy();
    expect(screen.queryByText("a1 remembers this")).toBeNull();
  });
});

describe("moving to a section", () => {
  beforeEach(() => {
    agentComputer.mockResolvedValue(null);
    agentBrowser.mockResolvedValue(null);
    agentRoutines.mockResolvedValue([]);
    agentMemory.mockResolvedValue("");
    configure();
  });

  it("opens each tab at its top, wherever the last one was left", async () => {
    const scrolled = vi.spyOn(Element.prototype, "scrollIntoView");
    try {
      const view = render(
        <AgentPanel section="memory" agent={card("a1", "Cook")} onEditProfile={vi.fn()} />,
      );
      view.rerender(
        <AgentPanel section="notebook" agent={card("a1", "Cook")} onEditProfile={vi.fn()} />,
      );
      await waitFor(() => expect(scrolled).toHaveBeenCalled());
      const last = scrolled.mock.contexts.at(-1) as Element;
      expect(last.id).toBe("agent-notebook");
      expect(last.parentElement?.firstElementChild).toBe(last);
      expect(scrolled).toHaveBeenLastCalledWith({ block: "start", behavior: "auto" });
    } finally {
      scrolled.mockRestore();
    }
  });

  it("jumps rather than glides when the operator asked for less motion", async () => {
    vi.stubGlobal("matchMedia", (query: string) => ({
      matches: query === "(prefers-reduced-motion: reduce)",
    }));
    const scrolled = vi.spyOn(Element.prototype, "scrollIntoView");
    try {
      render(<AgentPanel section="memory" agent={card("a1", "Cook")} onEditProfile={vi.fn()} />);
      await waitFor(() => expect(scrolled).toHaveBeenCalled());
      expect(scrolled).toHaveBeenLastCalledWith({ block: "start", behavior: "auto" });
    } finally {
      vi.unstubAllGlobals();
      scrolled.mockRestore();
    }
  });
});
