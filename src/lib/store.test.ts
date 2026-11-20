import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { anApproval } from "../test-fixtures";
import { expectRestart, forgetRestart } from "./link";
import type { AgentCard, Approval, Envelope, Running, UiEvent } from "./types";

vi.mock("./ipc", () => ({
  api: {
    listAgents: vi.fn(async () => [] as AgentCard[]),
    agentActivity: vi.fn(async () => ({})),
    agentLastActive: vi.fn(async () => ({})),
    capabilities: vi.fn(async () => ({})),
    getSettings: vi.fn(async () => null),
    channelMessages: vi.fn(async () => [] as Envelope[]),
    activityFeed: vi.fn(async () => [] as Envelope[]),
    conversationFlow: vi.fn(async () => [] as Envelope[]),
    usageSummary: vi.fn(async () => []),
    listGroups: vi.fn(async () => []),
    moveAgent: vi.fn(async () => null),
    setAgentPinned: vi.fn(async () => null),
    approvalStates: async () => ({ "q-1": "answered" }),
    approvalAnswers: async () => ({ "q-1": "Northwind" }),
    pendingApprovals: vi.fn(async (): Promise<Approval[]> => []),
    openEscalations: async () => [],
    listDecisions: async () => [],
    listOutcomes: async () => [],
    runningRuns: vi.fn(async (): Promise<Running[]> => []),
  },
  onRuntimeEvent: vi.fn(),
}));

vi.mock("./transport", async (original) => ({
  ...(await original<typeof import("./transport")>()),
  openExternal: vi.fn(async () => {}),
}));

const { useStore } = await import("./store");
const { api } = await import("./ipc");
const { DEFAULT_PREFS } = await import("./prefs");

function envelope(overrides: Partial<Envelope> = {}): Envelope {
  return {
    id: "m1",
    runId: "r1",
    channelId: "chef",
    from: { kind: "human" },
    to: { kind: "agent", id: "chef" },
    parts: [{ type: "text", text: "hello" }],
    trust: "operator",
    hop: 0,
    expectsReply: true,
    intent: "courtesy",
    cause: null,
    createdAt: 100,
    ...overrides,
  };
}

const AGENTS: AgentCard[] = [
  {
    id: "manager",
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    name: "Manager",
    avatar: "avocado",
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
  },
  {
    id: "chef",
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
    browserConsent: "open",
    name: "Chef",
    avatar: "chilli",
    color: "#e2674a",
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
  },
];

function reset(messages: Record<string, Envelope[] | undefined> = {}) {
  useStore.setState({
    ...useStore.getInitialState(),
    agents: AGENTS,
    activity: {},
    lastActive: {},
    settings: null,
    prefs: DEFAULT_PREFS,
    activeRun: {},
    selected: "chef",
    messages,
    streams: {},
    reasoning: {},
    trail: {},
    pulses: [],
    usage: {},
    pulse: {},
    banner: null,
    link: "up",
    lastHeard: null,
    railGroup: null,
  });
}

const APPROVALS = { "q-1": "answered" };
const ANSWERS = { "q-1": "Northwind" };

const apply = (event: UiEvent) => useStore.getState().applyEvent(event);

beforeEach(() => {
  reset();
  forgetRestart();
});

describe("the reset between tests", () => {
  it("leaves no slice behind from the test before", () => {
    useStore.setState({ pending: [anApproval({ agentId: "chef" })] });
    reset();
    expect(useStore.getState().pending).toEqual(useStore.getInitialState().pending);
  });
});

describe("the line to the host", () => {
  const gone = {
    kind: "unreachable",
    message: "lost the connection to this host at http://box.local:8787. It may be restarting.",
  };
  const state = () => useStore.getState();

  it("is the banner a failed call would raise, and remembers when the host last spoke", () => {
    state().linkDown(gone, 1_700_000_000_000);
    expect(state().link).toBe("down");
    expect(state().lastHeard).toBe(1_700_000_000_000);
    expect(state().banner).toMatchObject({ tone: "error", fix: "host" });
    expect(state().banner?.text).toMatch(/^Lost the connection to this host/);
    expect(state().banner?.text).toMatch(/restarting\.$/);
  });

  it("takes back only the sentence about the host when it answers again", () => {
    state().linkDown(gone, 1_700_000_000_000);
    state().showFailure({ kind: "storage", message: "the disk is full" });
    state().linkUp();
    expect(state().link).toBe("up");
    expect(state().lastHeard).toBeNull();
    expect(state().banner?.text).toBe("The disk is full.");
  });

  it("says nothing when an update is restarting the host on purpose", () => {
    expectRestart();
    state().linkDown(gone, 1_700_000_000_000);
    expect(state().link).toBe("up");
    expect(state().lastHeard).toBeNull();
    expect(state().banner).toBeNull();
  });
});

describe("messageAppended", () => {
  it("appends to a loaded channel", () => {
    reset({ chef: [] });
    apply({ type: "messageAppended", message: envelope() });
    expect(useStore.getState().messages.chef).toHaveLength(1);
  });

  it("leaves an unloaded channel alone", () => {
    apply({ type: "messageAppended", message: envelope() });
    expect(useStore.getState().messages.chef).toBeUndefined();
  });

  it("ignores a message it already has", () => {
    reset({ chef: [] });
    apply({ type: "messageAppended", message: envelope() });
    apply({ type: "messageAppended", message: envelope() });
    expect(useStore.getState().messages.chef).toHaveLength(1);
  });

  it("keeps a channel ordered when messages arrive late", () => {
    reset({ chef: [] });
    apply({ type: "messageAppended", message: envelope({ id: "b", createdAt: 200 }) });
    apply({ type: "messageAppended", message: envelope({ id: "a", createdAt: 100 }) });
    expect(useStore.getState().messages.chef?.map((m) => m.id)).toEqual(["a", "b"]);
  });

  it("orders deterministically when timestamps collide", () => {
    reset({ chef: [] });
    apply({ type: "messageAppended", message: envelope({ id: "z", createdAt: 100 }) });
    apply({ type: "messageAppended", message: envelope({ id: "a", createdAt: 100 }) });
    expect(useStore.getState().messages.chef?.map((m) => m.id)).toEqual(["a", "z"]);
  });
});

describe("rail pulses", () => {
  it("fires for agent-to-agent traffic in the sender's color", () => {
    apply({
      type: "messageAppended",
      message: envelope({
        from: { kind: "agent", id: "manager" },
        to: { kind: "agent", id: "chef" },
      }),
    });
    const [pulse] = useStore.getState().pulses;
    expect(pulse).toMatchObject({ from: "manager", to: "chef", color: "#c7d96b" });
  });

  it("fires even when neither channel is open", () => {
    expect(useStore.getState().messages.chef).toBeUndefined();
    apply({
      type: "messageAppended",
      message: envelope({
        from: { kind: "agent", id: "manager" },
        to: { kind: "agent", id: "chef" },
      }),
    });
    expect(useStore.getState().pulses).toHaveLength(1);
  });

  it("does not fire for operator messages", () => {
    apply({ type: "messageAppended", message: envelope() });
    expect(useStore.getState().pulses).toHaveLength(0);
  });

  it("is dismissed by id", () => {
    apply({
      type: "messageAppended",
      message: envelope({
        from: { kind: "agent", id: "manager" },
        to: { kind: "agent", id: "chef" },
      }),
    });
    const id = useStore.getState().pulses[0]!.id;
    useStore.getState().dismissPulse(id);
    expect(useStore.getState().pulses).toHaveLength(0);
  });
});

describe("streaming", () => {
  const started: UiEvent = {
    type: "streamStarted",
    messageId: "s1",
    channelId: "chef",
    agentId: "chef",
    runId: "r1",
    to: { kind: "human" },
  };

  it("accumulates deltas in order", () => {
    apply(started);
    apply({ type: "streamDelta", messageId: "s1", channelId: "chef", text: "Hel" });
    apply({ type: "streamDelta", messageId: "s1", channelId: "chef", text: "lo" });
    expect(useStore.getState().streams.s1?.text).toBe("Hello");
  });

  it("ignores deltas for a stream that never started", () => {
    apply({ type: "streamDelta", messageId: "ghost", channelId: "chef", text: "x" });
    expect(useStore.getState().streams.ghost).toBeUndefined();
  });

  it("keeps concurrent streams separate", () => {
    apply(started);
    apply({
      type: "streamStarted",
      messageId: "s2",
      channelId: "manager",
      agentId: "manager",
      runId: "r1",
      to: { kind: "human" },
    });
    apply({ type: "streamDelta", messageId: "s1", channelId: "chef", text: "chef" });
    apply({ type: "streamDelta", messageId: "s2", channelId: "manager", text: "manager" });

    expect(useStore.getState().streams.s1?.text).toBe("chef");
    expect(useStore.getState().streams.s2?.text).toBe("manager");
  });

  it("remembers who a stream is for", () => {
    apply({
      type: "streamStarted",
      messageId: "s3",
      channelId: "manager",
      agentId: "chef",
      runId: "r1",
      to: { kind: "agent", id: "manager" },
    });
    expect(useStore.getState().streams.s3?.to).toEqual({ kind: "agent", id: "manager" });
  });

  it("clears the buffer when the stream ends", () => {
    apply(started);
    apply({ type: "streamEnded", messageId: "s1", channelId: "chef" });
    expect(useStore.getState().streams.s1).toBeUndefined();
  });
});

describe("what an agent is thinking", () => {
  const started: UiEvent = {
    type: "streamStarted",
    messageId: "s1",
    channelId: "manager",
    agentId: "chef",
    runId: "r1",
    to: { kind: "agent", id: "manager" },
  };

  it("is filed under the agent doing the thinking, not the channel it writes to", () => {
    apply(started);
    apply({ type: "reasoningDelta", messageId: "s1", text: "weighing it up" });
    expect(useStore.getState().reasoning.chef).toBe("weighing it up");
    expect(useStore.getState().reasoning.manager).toBeUndefined();
  });

  it("is dropped when the turn ends", () => {
    apply(started);
    apply({ type: "reasoningDelta", messageId: "s1", text: "weighing it up" });
    apply({ type: "streamEnded", messageId: "s1", channelId: "manager" });
    expect(useStore.getState().reasoning.chef).toBeUndefined();
  });

  it("is dropped when a failed call reopens under a new id", () => {
    apply(started);
    apply({ type: "reasoningDelta", messageId: "s1", text: "half a thought" });
    apply({ type: "streamEnded", messageId: "s1", channelId: "manager" });
    apply({ ...started, messageId: "s2" });
    expect(useStore.getState().reasoning.chef).toBeUndefined();
  });

  it("ignores a thought for a stream that never started", () => {
    apply({ type: "reasoningDelta", messageId: "ghost", text: "x" });
    expect(useStore.getState().reasoning).toEqual({});
  });

  it("never lands in the transcript", () => {
    apply(started);
    apply({ type: "reasoningDelta", messageId: "s1", text: "weighing it up" });
    expect(JSON.stringify(useStore.getState().messages)).not.toContain("weighing");
    expect(useStore.getState().streams.s1?.text).toBe("");
  });
});

describe("what an agent is reaching for", () => {
  it("keeps reasoning boundaries aligned when the oldest text exceeds the buffer", () => {
    apply({
      type: "streamStarted",
      messageId: "bounded",
      channelId: "chef",
      agentId: "chef",
      runId: "r1",
      to: { kind: "human" },
    });
    apply({ type: "reasoningDelta", messageId: "bounded", text: "a".repeat(199_999) });
    apply({
      type: "toolStarted",
      messageId: "bounded",
      callId: "c",
      name: "directory",
      arguments: {},
    });
    expect(useStore.getState().trail.chef?.[0]?.reasoningOffset).toBe(199_999);
    apply({ type: "reasoningDelta", messageId: "bounded", text: "bbb" });
    expect(useStore.getState().reasoning.chef?.length).toBe(200_000);
    const boundary = useStore.getState().trail.chef?.[0]?.reasoningOffset;
    expect(boundary).toBe(199_997);
    expect(useStore.getState().reasoning.chef?.slice(boundary)).toBe("bbb");
    apply({ type: "reasoningDelta", messageId: "bounded", text: "c".repeat(200_000) });
    expect(useStore.getState().trail.chef?.[0]?.reasoningOffset).toBe(0);
  });

  const started: UiEvent = {
    type: "streamStarted",
    messageId: "s1",
    channelId: "manager",
    agentId: "chef",
    runId: "r1",
    to: { kind: "agent", id: "manager" },
  };
  const opened: UiEvent = {
    type: "toolStarted",
    messageId: "s1",
    callId: "call_1",
    name: "run_command",
    arguments: { command: "npm test" },
  };
  const cameBack = {
    type: "toolFinished",
    messageId: "s1",
    callId: "call_1",
    part: {
      type: "toolCall",
      name: "run_command",
      arguments: { command: "npm test" },
      outcome: { status: "ok", summary: "exit 0" },
    },
  } as const satisfies UiEvent;

  it("is filed under the agent making the call, like the thought beside it", () => {
    apply(started);
    apply(opened);
    expect(useStore.getState().trail.chef?.[0]?.name).toBe("run_command");
    expect(useStore.getState().trail.manager).toBeUndefined();
  });

  it("has no record of the call until it comes back", () => {
    apply(started);
    apply(opened);
    expect(useStore.getState().trail.chef?.[0]?.done).toBeNull();

    apply(cameBack);
    expect(useStore.getState().trail.chef?.[0]?.done).toEqual(cameBack.part);
  });

  it("keeps the whole part, so a live chip draws what a recorded one does", () => {
    apply(started);
    apply(opened);
    apply({
      type: "toolFinished",
      messageId: "s1",
      callId: "call_1",
      part: {
        type: "toolCall",
        name: "update_memory",
        arguments: { content: "Smith handles verification." },
        outcome: { status: "ok", summary: "Memory saved." },
        replaced: "Jones handles verification.",
      },
    });

    expect(useStore.getState().trail.chef?.[0]?.done?.replaced).toBe("Jones handles verification.");
  });

  it("keeps two calls of one kind apart by the id the provider gave them", () => {
    apply(started);
    apply(opened);
    apply({ ...opened, callId: "call_2" });
    apply({ ...cameBack, callId: "call_2" });

    const held = useStore.getState().trail.chef;
    expect(held?.[0]?.done).toBeNull();
    expect(held?.[1]?.done).toEqual(cameBack.part);
  });

  it("is dropped when the turn ends", () => {
    apply(started);
    apply(opened);
    apply({ type: "streamEnded", messageId: "s1", channelId: "manager" });
    expect(useStore.getState().trail.chef).toBeUndefined();
  });

  it("starts again when a failed call reopens under a new id", () => {
    apply(started);
    apply(opened);
    apply({ ...started, messageId: "s2" });
    expect(useStore.getState().trail.chef).toBeUndefined();
  });

  it("ignores a call for a stream that never started", () => {
    apply(opened);
    expect(useStore.getState().trail).toEqual({});
  });

  it("never lands in the transcript", () => {
    apply(started);
    apply(opened);
    expect(JSON.stringify(useStore.getState().messages)).not.toContain("npm test");
  });
});

describe("sidebar ordering", () => {
  it("records activity for both ends of a message", () => {
    apply({
      type: "messageAppended",
      message: envelope({
        from: { kind: "agent", id: "manager" },
        to: { kind: "agent", id: "chef" },
        createdAt: 500,
      }),
    });
    expect(useStore.getState().lastActive.manager).toBe(500);
    expect(useStore.getState().lastActive.chef).toBe(500);
  });

  it("keeps the newest timestamp", () => {
    apply({ type: "messageAppended", message: envelope({ id: "a", createdAt: 900 }) });
    apply({ type: "messageAppended", message: envelope({ id: "b", createdAt: 100 }) });
    expect(useStore.getState().lastActive.chef).toBe(900);
  });
});

describe("the group the rail is inside", () => {
  const RESEARCH = "00000000-0000-4000-8000-000000000002";

  it("is left alone while the channel being opened is in it", async () => {
    reset({ chef: [] });
    await useStore.getState().focusGroup(AGENTS[0]!.groupId);
    await useStore.getState().select("manager");
    expect(useStore.getState().railGroup).toBe(AGENTS[0]!.groupId);
  });

  it("follows the channel being opened into the crew it belongs to", async () => {
    reset({ chef: [] });
    useStore.setState({
      agents: [AGENTS[0]!, { ...AGENTS[1]!, groupId: RESEARCH }],
    });
    await useStore.getState().focusGroup(RESEARCH);

    await useStore.getState().select("manager");
    expect(useStore.getState().railGroup).toBe(AGENTS[0]!.groupId);
  });

  it("follows a jump to a message in another crew's channel", async () => {
    reset({ chef: [] });
    useStore.setState({
      agents: [AGENTS[0]!, { ...AGENTS[1]!, groupId: RESEARCH }],
    });
    await useStore.getState().focusGroup(RESEARCH);

    await useStore.getState().openMessage("manager", "m-old");
    expect(useStore.getState().railGroup).toBe(AGENTS[0]!.groupId);
  });

  it("stays on the overview when a channel is opened from it", async () => {
    reset({ chef: [] });
    useStore.setState({
      agents: [AGENTS[0]!, { ...AGENTS[1]!, groupId: RESEARCH }],
      railGroup: null,
    });

    await useStore.getState().select("manager");
    expect(useStore.getState().railGroup).toBeNull();
  });

  it("stays where it is when the channel belongs to nobody", async () => {
    reset({ chef: [] });
    await useStore.getState().focusGroup(AGENTS[0]!.groupId);

    await useStore.getState().select("ghost");
    expect(useStore.getState().railGroup).toBe(AGENTS[0]!.groupId);
  });
});

describe("the channel open when the rail goes inside a crew", () => {
  const RESEARCH = "00000000-0000-4000-8000-000000000002";

  function twoCrews() {
    reset({ chef: [] });
    useStore.setState({
      agents: [AGENTS[1]!, { ...AGENTS[1]!, id: "chef-research", groupId: RESEARCH }],
    });
  }

  it("keeps the conversation open when management switches groups", async () => {
    twoCrews();
    useStore.setState({ selected: "chef-research" });

    await useStore.getState().focusGroup(AGENTS[1]!.groupId);

    expect(useStore.getState().selected).toBe("chef-research");
    expect(useStore.getState().railGroup).toBe(AGENTS[1]!.groupId);
  });

  it("stays open when it belongs to the crew being opened", async () => {
    twoCrews();
    useStore.setState({ selected: "chef-research" });

    await useStore.getState().focusGroup(RESEARCH);

    expect(useStore.getState().selected).toBe("chef-research");
    expect(useStore.getState().railGroup).toBe(RESEARCH);
  });

  it("stays open on the way back out to the overview, which draws everybody", async () => {
    twoCrews();
    useStore.setState({ selected: "chef-research" });
    await useStore.getState().focusGroup(RESEARCH);

    await useStore.getState().focusGroup(null);

    expect(useStore.getState().selected).toBe("chef-research");
    expect(useStore.getState().railGroup).toBeNull();
  });
});

describe("activity", () => {
  it("waits for the latest roster even when its first read is superseded twice", async () => {
    const initial = useStore.getState();
    let latest!: (rows: AgentCard[]) => void;
    let earlier!: (rows: AgentCard[]) => void;
    vi.mocked(api.listAgents)
      .mockResolvedValueOnce([])
      .mockReturnValueOnce(
        new Promise((resolve) => {
          earlier = resolve;
        }),
      )
      .mockReturnValueOnce(
        new Promise((resolve) => {
          latest = resolve;
        }),
      );
    const ready = vi.fn();
    const startup = initial.bootstrap().then(ready);
    const middle = initial.refreshAgents();
    const newer = initial.refreshAgents();
    earlier([AGENTS[0]!]);
    await middle;
    await new Promise<void>((resolve) => queueMicrotask(resolve));
    try {
      expect(ready).not.toHaveBeenCalled();
    } finally {
      latest(AGENTS);
      await Promise.all([startup, newer]);
    }
    expect(useStore.getState().agents).toEqual(AGENTS);
  });

  it("does not finish bootstrap before a newer question snapshot is applied", async () => {
    const request = anApproval({ agentId: "chef" });
    let first!: (rows: Approval[]) => void;
    let latest!: (rows: Approval[]) => void;
    vi.mocked(api.pendingApprovals)
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            first = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            latest = resolve;
          }),
      );
    const ready = vi.fn();
    const startup = useStore.getState().bootstrap().then(ready);
    const newer = useStore.getState().refreshApprovals();
    first([request]);
    try {
      await waitFor(() => expect(useStore.getState().agents).toEqual([]));
      expect(ready).not.toHaveBeenCalled();
    } finally {
      latest([request]);
      await Promise.all([startup, newer]);
    }
    expect(useStore.getState().pending).toEqual([request]);
  });

  it("waits for the live snapshot read when it supersedes the startup read", async () => {
    const row = {
      runId: "r1",
      agentId: "chef",
      groupId: AGENTS[1]!.groupId,
      startedAt: 1,
      note: null,
    };
    let first!: (rows: Running[]) => void;
    let latest!: (rows: Running[]) => void;
    vi.mocked(api.runningRuns)
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            first = resolve;
          }),
      )
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            latest = resolve;
          }),
      );
    const startup = useStore.getState().refreshRunning();
    const snapshot = useStore.getState().refreshRunning();
    first([]);
    const settled = vi.fn();
    void startup.then(settled);
    await Promise.resolve();
    await Promise.resolve();
    expect(settled).not.toHaveBeenCalled();
    latest([row]);
    await startup;
    await snapshot;
    expect(useStore.getState().running).toEqual([row]);
  });

  it("records the latest state per agent", () => {
    apply({ type: "activityChanged", agentId: "chef", activity: { state: "thinking" } });
    expect(useStore.getState().activity.chef).toEqual({ state: "thinking" });

    apply({ type: "activityChanged", agentId: "chef", activity: { state: "queued", depth: 2 } });
    expect(useStore.getState().activity.chef).toEqual({ state: "queued", depth: 2 });
  });

  it("does not disturb other agents", () => {
    apply({ type: "activityChanged", agentId: "chef", activity: { state: "thinking" } });
    apply({ type: "activityChanged", agentId: "manager", activity: { state: "idle" } });
    expect(useStore.getState().activity.chef).toEqual({ state: "thinking" });
  });
});

describe("channelsCleared", () => {
  it("drops what it is holding for the cleared channels", async () => {
    reset({
      chef: [envelope()],
      manager: [envelope({ channelId: "manager" })],
    });

    apply({ type: "channelsCleared", agents: ["chef"] });

    expect(useStore.getState().messages.chef).toBeUndefined();
    expect(useStore.getState().messages.manager).toHaveLength(1);
  });

  it("reads the open channel again rather than leaving it blank", async () => {
    reset({ chef: [envelope()] });
    apply({ type: "channelsCleared", agents: ["chef"] });

    const { channelMessages } = (await import("./ipc")).api as unknown as {
      channelMessages: ReturnType<typeof vi.fn>;
    };
    expect(channelMessages).toHaveBeenCalledWith("chef", 300, undefined);
  });

  it("sends the panels holding the two stores back for them", () => {
    reset({ chef: [envelope()] });
    useStore.setState({ memoryVersion: {}, workingNotesVersion: {} });

    apply({ type: "channelsCleared", agents: ["chef"] });

    expect(useStore.getState().memoryVersion.chef).toBe(1);
    expect(useStore.getState().workingNotesVersion.chef).toBe(1);
    expect(useStore.getState().workingNotesVersion.manager).toBeUndefined();
  });
});

describe("openMessage", () => {
  it("keeps the recent conversation around a result", async () => {
    reset({ chef: [] });
    const { channelMessages } = (await import("./ipc")).api;
    const handoff = envelope({ id: "handoff" });
    const result = envelope({ id: "result" });
    vi.mocked(channelMessages).mockResolvedValueOnce([handoff, result]);

    await useStore.getState().openMessage("chef", result.id, "result");

    expect(channelMessages).toHaveBeenCalledExactlyOnceWith("chef", 300, undefined);
    expect(useStore.getState().messages.chef).toEqual([handoff, result]);
    expect(useStore.getState().focused).toBe(result.id);
  });

  it("can still reach a result outside the recent conversation", async () => {
    reset({ chef: [] });
    const { channelMessages } = (await import("./ipc")).api;
    const newer = envelope({ id: "newer" });
    const result = envelope({ id: "result" });
    vi.mocked(channelMessages)
      .mockResolvedValueOnce([newer])
      .mockResolvedValueOnce([result, newer]);

    await useStore.getState().openMessage("chef", result.id, "result");

    expect(channelMessages).toHaveBeenNthCalledWith(1, "chef", 300, undefined);
    expect(channelMessages).toHaveBeenNthCalledWith(2, "chef", 300, result.id);
    expect(useStore.getState().messages.chef).toEqual([result, newer]);
  });

  it("asks for a window wide enough to hold the message", async () => {
    reset({ chef: [] });
    await useStore.getState().openMessage("chef", "m-old");

    const { channelMessages } = (await import("./ipc")).api as unknown as {
      channelMessages: ReturnType<typeof vi.fn>;
    };
    expect(channelMessages).toHaveBeenCalledWith("chef", 300, "m-old");
  });

  it("marks the message before the read comes back", async () => {
    reset({ chef: [] });
    const pending = useStore.getState().openMessage("chef", "m-old");

    expect(useStore.getState().selected).toBe("chef");
    expect(useStore.getState().focused).toBe("m-old");
    await pending;
  });

  it("drops the mark when the operator moves on by hand", async () => {
    reset({ chef: [] });
    await useStore.getState().openMessage("chef", "m-old");
    await useStore.getState().select("manager");

    expect(useStore.getState().focused).toBeNull();
  });
});

describe("reconnecting to a running workspace", () => {
  it("waits for the persisted reply after a truncated snapshot", () => {
    reset({ chef: [] });
    apply({
      type: "liveSnapshot",
      activity: { chef: { state: "thinking" } },
      streams: {
        current: {
          agentId: "chef",
          channelId: "chef",
          runId: "running",
          to: { kind: "human" },
          text: "",
          truncated: true,
        },
      },
    });
    apply({ type: "streamDelta", messageId: "current", channelId: "chef", text: "suffix" });
    expect(useStore.getState().streams.current?.text).toBe("");
    expect(useStore.getState().activeRun.chef).toBe("running");
    apply({
      type: "messageAppended",
      message: envelope({ id: "current", parts: [{ type: "text", text: "complete reply" }] }),
    });
    apply({ type: "streamEnded", messageId: "current", channelId: "chef" });
    expect(useStore.getState().messages.chef?.find((m) => m.id === "current")?.parts).toEqual([
      { type: "text", text: "complete reply" },
    ]);
    expect(useStore.getState().streams.current).toBeUndefined();
  });

  it("continues an under-limit snapshot and a legacy snapshot without a truncation field", () => {
    for (const truncated of [false, undefined]) {
      apply({
        type: "liveSnapshot",
        activity: {},
        streams: {
          current: {
            agentId: "chef",
            channelId: "chef",
            runId: "running",
            to: { kind: "human" },
            text: "prefix",
            truncated,
          },
        },
      });
      apply({ type: "streamDelta", messageId: "current", channelId: "chef", text: " suffix" });
      expect(useStore.getState().streams.current?.text).toBe("prefix suffix");
    }
  });

  it("replaces obsolete streams and restores the run that can be stopped", () => {
    useStore.setState({
      streams: {
        old: { agentId: "chef", channelId: "chef", to: { kind: "human" }, text: "stale" },
      },
      activeRun: { chef: "finished" },
      reasoning: { chef: "old thought" },
    });
    apply({
      type: "liveSnapshot",
      activity: { manager: { state: "thinking" } },
      streams: {
        current: {
          agentId: "manager",
          channelId: "manager",
          runId: "running",
          to: { kind: "human" },
          text: "Already written",
        },
      },
    });
    expect(useStore.getState().activeRun).toEqual({ manager: "running" });
    expect(useStore.getState().streams.old).toBeUndefined();
    expect(useStore.getState().reasoning).toEqual({});
    apply({ type: "streamDelta", messageId: "current", channelId: "manager", text: " plus live" });
    expect(useStore.getState().streams.current?.text).toBe("Already written plus live");
    apply({
      type: "liveSnapshot",
      activity: { manager: { state: "idle" } },
      streams: {},
    });
    expect(useStore.getState().activeRun).toEqual({});
    expect(useStore.getState().streams).toEqual({});
  });

  it("keeps a message arriving while a transcript refresh is in flight", async () => {
    const { api } = await import("./ipc");
    let finish!: (rows: Envelope[]) => void;
    vi.mocked(api.channelMessages).mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    reset({ chef: [] });
    const loading = useStore.getState().loadChannel("chef");
    apply({ type: "messageAppended", message: envelope({ id: "new", createdAt: 200 }) });
    finish([envelope()]);
    await loading;
    expect(useStore.getState().messages.chef?.map((m) => m.id)).toEqual(["m1", "new"]);
  });

  it("keeps a message arriving while a channel's first read is in flight", async () => {
    const { api } = await import("./ipc");
    let finish!: (rows: Envelope[]) => void;
    vi.mocked(api.channelMessages).mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    reset({});
    const loading = useStore.getState().loadChannel("chef");
    apply({ type: "messageAppended", message: envelope({ id: "new", createdAt: 200 }) });
    finish([envelope()]);
    await loading;
    expect(useStore.getState().messages.chef?.map((m) => m.id)).toEqual(["m1", "new"]);
  });

  it("does not commit a channel snapshot after its connection is invalidated", async () => {
    const { api } = await import("./ipc");
    let finish!: (rows: Envelope[]) => void;
    vi.mocked(api.channelMessages).mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    reset({ chef: [envelope({ id: "current" })] });
    let valid = true;
    const loading = useStore.getState().loadChannel("chef", undefined, () => valid);
    valid = false;
    finish([envelope({ id: "stale" })]);
    await loading;
    expect(useStore.getState().messages.chef?.map((m) => m.id)).toEqual(["current"]);
  });

  it("does not restore a cleared channel from an older read", async () => {
    const { api } = await import("./ipc");
    let finish!: (rows: Envelope[]) => void;
    vi.mocked(api.channelMessages).mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    reset({ chef: [envelope()] });
    const loading = useStore.getState().loadChannel("chef");
    useStore.setState({ selected: "manager" });
    apply({ type: "channelsCleared", agents: ["chef"] });
    finish([envelope()]);
    await loading;
    expect(useStore.getState().messages.chef).toBeUndefined();
  });

  it("does not commit an event-triggered roster read after disconnect", async () => {
    const { api } = await import("./ipc");
    let finish!: (agents: AgentCard[]) => void;
    vi.mocked(api.listAgents).mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    let valid = true;
    const loading = useStore.getState().refreshAgents(() => valid);
    valid = false;
    finish([]);
    await loading;
    expect(useStore.getState().agents).toEqual(AGENTS);
  });

  it("reconciles the host snapshot and an event into the final visible channel", async () => {
    const { api, onRuntimeEvent } = await import("./ipc");
    const { useRuntime } = await import("./runtime");
    let emit!: (event: UiEvent) => void;
    let opened!: () => void;
    vi.mocked(onRuntimeEvent).mockImplementation(async (event, open) => {
      emit = event;
      opened = open!;
      return () => {};
    });
    vi.mocked(api.listAgents).mockResolvedValue(AGENTS);
    vi.mocked(api.channelMessages).mockClear();
    const view = renderHook(() => useRuntime());
    await waitFor(() => expect(view.result.current).toBe(true));
    let finish!: (rows: Envelope[]) => void;
    vi.mocked(api.channelMessages)
      .mockReturnValueOnce(
        new Promise((resolve) => {
          finish = resolve;
        }),
      )
      .mockResolvedValueOnce([envelope(), envelope({ id: "new", createdAt: 200 })]);
    act(() => opened());
    await waitFor(() => expect(finish).toBeTypeOf("function"));
    act(() => emit({ type: "messageAppended", message: envelope({ id: "new", createdAt: 200 }) }));
    await act(async () => finish([envelope()]));
    await waitFor(() => expect(api.channelMessages).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(useStore.getState().link).toBe("up"));
    expect(useStore.getState().messages.chef?.map((m) => m.id)).toEqual(["m1", "new"]);
    view.unmount();
    vi.mocked(api.listAgents).mockResolvedValue([]);
  });
});

describe("a sign-in page", () => {
  it("opens in the window that asked for it, and in no other", async () => {
    const { PAGE_ID } = await import("./transport");
    useStore.setState({ handoff: null });
    const apply = (client: string | null | undefined) =>
      useStore.getState().applyEvent({ type: "openUrl", url: "https://signin.example", client });
    apply("another-window");
    expect(useStore.getState().handoff).toBeNull();
    apply(PAGE_ID);
    expect(useStore.getState().handoff).toBe("https://signin.example");
    useStore.setState({ handoff: null });
    apply(undefined);
    expect(useStore.getState().handoff).toBe("https://signin.example");
  });
});

describe("a failure on the banner", () => {
  beforeEach(() => {
    useStore.setState({ banner: null });
  });

  it("offers what can be done about it when the host is the thing that is gone", () => {
    useStore.getState().showFailure({
      kind: "unreachable",
      message:
        "could not reach this workspace at http://127.0.0.1:8787. Its agents keep working either way",
    });
    expect(useStore.getState().banner).toEqual({
      tone: "error",
      text: "Could not reach this workspace at http://127.0.0.1:8787. Its agents keep working either way.",
      fix: "host",
    });
  });

  it("offers nothing but the sentence for everything else", () => {
    for (const error of [
      { kind: "storage", message: "the disk is full" },
      { kind: "validation", message: "there are two Pips" },
      new Error("something else entirely"),
    ]) {
      useStore.getState().showFailure(error);
      expect(useStore.getState().banner?.fix).toBeUndefined();
      expect(useStore.getState().banner?.tone).toBe("error");
    }
  });
});

describe("what a settled question is read back as", () => {
  it("keeps the answer beside the state it came with", async () => {
    await useStore.getState().refreshApprovals();
    expect(useStore.getState().approvals).toEqual(APPROVALS);
    expect(useStore.getState().answers).toEqual(ANSWERS);
  });
});
