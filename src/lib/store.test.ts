import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AgentCard, Envelope, UiEvent } from "./types";

// The store talks to Tauri, which does not exist in a test runner. Only the
// transport is mocked; the reducer logic under test is the real one.
vi.mock("./ipc", () => ({
  api: {
    listAgents: vi.fn(async () => [] as AgentCard[]),
    agentActivity: vi.fn(async () => ({})),
    getSettings: vi.fn(async () => null),
    channelMessages: vi.fn(async () => [] as Envelope[]),
    activityFeed: vi.fn(async () => [] as Envelope[]),
    conversationFlow: vi.fn(async () => [] as Envelope[]),
    usageSummary: vi.fn(async () => []),
    listGroups: vi.fn(async () => []),
    listRepositories: vi.fn(async () => []),
    moveAgent: vi.fn(async () => null),
    setAgentPinned: vi.fn(async () => null),
  },
  onRuntimeEvent: vi.fn(),
}));

const { ACTIVITY_CHANNEL, useStore } = await import("./store");
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
    repositoryId: null,
    name: "Manager",
    avatar: "avocado",
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
  },
  {
    id: "chef",
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    repositoryId: null,
    name: "Chef",
    avatar: "chilli",
    color: "#e2674a",
    model: "m",
    systemPrompt: "",
    skills: [],
    lifecycle: "active",
    pinned: false,
    railOrder: 0,
    version: 1,
    createdAt: 0,
    updatedAt: 0,
  },
];

function reset(messages: Record<string, Envelope[] | undefined> = {}) {
  useStore.setState({
    agents: AGENTS,
    activity: {},
    lastActive: {},
    settings: null,
    // Named explicitly for the same reason as every field above it: zustand's
    // setState merges, so a slice left out of this list leaks into whichever
    // test runs next.
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
    railGroup: null,
  });
}

const apply = (event: UiEvent) => useStore.getState().applyEvent(event);

beforeEach(() => reset());

describe("messageAppended", () => {
  it("appends to a loaded channel", () => {
    reset({ chef: [] });
    apply({ type: "messageAppended", message: envelope() });
    expect(useStore.getState().messages.chef).toHaveLength(1);
  });

  it("leaves an unloaded channel alone", () => {
    // Appending into a channel that was never fetched would produce a
    // transcript with a hole in it the first time it is opened.
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

  it("puts the whole conversation on the flow board", () => {
    // Including the operator's own messages: a flow that starts at the first
    // agent-to-agent message hides who set it off.
    reset({ chef: [], [ACTIVITY_CHANNEL]: [] });
    apply({ type: "messageAppended", message: envelope({ id: "a" }) });
    apply({
      type: "messageAppended",
      message: envelope({
        id: "b",
        from: { kind: "agent", id: "manager" },
        to: { kind: "agent", id: "chef" },
      }),
    });
    expect(useStore.getState().messages[ACTIVITY_CHANNEL]).toHaveLength(2);
  });

  it("keeps private activity records off the flow board", () => {
    // An agent's own tool trail is bookkeeping, not a message between two
    // participants, so it has no arrow to draw.
    reset({ chef: [], [ACTIVITY_CHANNEL]: [] });
    apply({
      type: "messageAppended",
      message: envelope({ from: { kind: "agent", id: "chef" }, to: { kind: "system" } }),
    });
    expect(useStore.getState().messages[ACTIVITY_CHANNEL]).toHaveLength(0);
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
    // The pulse comes from the event, not from a channel read, which is what
    // lets you watch a cascade you are not looking at.
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
    // Out-of-order or post-teardown deltas must not resurrect a bubble.
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
    // A peer-bound stream is announced rather than rendered as text, so the
    // destination has to survive into the buffer.
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
    // Chef answering Manager streams into Manager's channel. The operator
    // watching Chef work is reading Chef's.
    apply(started);
    apply({ type: "reasoningDelta", messageId: "s1", text: "weighing it up" });
    expect(useStore.getState().reasoning.chef).toBe("weighing it up");
    expect(useStore.getState().reasoning.manager).toBeUndefined();
  });

  it("is dropped when the turn ends", () => {
    // The whole contract: it is visible while it happens and gone afterward.
    apply(started);
    apply({ type: "reasoningDelta", messageId: "s1", text: "weighing it up" });
    apply({ type: "streamEnded", messageId: "s1", channelId: "manager" });
    expect(useStore.getState().reasoning.chef).toBeUndefined();
  });

  it("is dropped when a failed call reopens under a new id", () => {
    // Anything already thought belongs to the attempt that broke, exactly like
    // the text that was already on screen.
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
    // It is not something anybody said, so no channel may hold it.
    apply(started);
    apply({ type: "reasoningDelta", messageId: "s1", text: "weighing it up" });
    expect(JSON.stringify(useStore.getState().messages)).not.toContain("weighing");
    expect(useStore.getState().streams.s1?.text).toBe("");
  });
});

describe("what an agent is reaching for", () => {
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
    // Which is the whole reason it is reported before the call rather than
    // after it: a command can sit for a minute, and that minute is the one the
    // operator cannot otherwise account for.
    apply(started);
    apply(opened);
    expect(useStore.getState().trail.chef?.[0]?.done).toBeNull();

    apply(cameBack);
    expect(useStore.getState().trail.chef?.[0]?.done).toEqual(cameBack.part);
  });

  it("keeps the whole part, so a live chip draws what a recorded one does", () => {
    // Not the outcome alone. A memory rewrite carries what it overwrote and
    // nothing outside the runtime could supply it, so a chip assembled here
    // from the fields somebody thought to list would quietly stop showing it.
    apply(started);
    apply(opened);
    apply({
      type: "toolFinished",
      messageId: "s1",
      callId: "call_1",
      part: {
        type: "toolCall",
        name: "update_notes",
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
    // The same contract as the thinking, from the same event: what a turn did
    // is the message that lands at the end of it, and the transcript draws
    // these chips again from that.
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
    // A message an agent sends is filed in the recipient's channel, so tracking
    // the channel alone would leave the sender looking idle.
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

  it("is left alone by the activity feed, which belongs to no group", async () => {
    reset({ chef: [] });
    await useStore.getState().focusGroup(AGENTS[0]!.groupId);
    await useStore.getState().select(ACTIVITY_CHANNEL);
    expect(useStore.getState().railGroup).toBe(AGENTS[0]!.groupId);
  });

  it("follows the channel being opened into the crew it belongs to", async () => {
    // A search hit or a click on the flow board can land anywhere, and a rail
    // still showing one crew while the pane shows a member of another has the
    // open channel nowhere on it. It used to drop out to the overview, which
    // was the only view that could draw every row while the crews lived in a
    // strip inside the rail. They have a column of their own now, so the crew
    // the operator has landed in is on screen either way and following it moves
    // one lit circle instead of rebuilding the rail.
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

  // The overview draws everybody, so there is nothing to repair and no reason
  // to narrow the rail to one crew behind a click that did not ask for one.
  it("stays on the overview when a channel is opened from it", async () => {
    reset({ chef: [] });
    useStore.setState({
      agents: [AGENTS[0]!, { ...AGENTS[1]!, groupId: RESEARCH }],
      railGroup: null,
    });

    await useStore.getState().select("manager");
    expect(useStore.getState().railGroup).toBeNull();
  });

  // An agent the roster no longer has says nothing about which crew to be in,
  // and guessing at one would move the rail away from a crew that is fine.
  it("stays where it is when the channel belongs to nobody", async () => {
    reset({ chef: [] });
    await useStore.getState().focusGroup(AGENTS[0]!.groupId);

    await useStore.getState().select("ghost");
    expect(useStore.getState().railGroup).toBe(AGENTS[0]!.groupId);
  });
});

describe("the channel open when the rail goes inside a crew", () => {
  const RESEARCH = "00000000-0000-4000-8000-000000000002";

  /** The same crew twice, with a namesake in each. */
  function twoCrews() {
    reset({ chef: [] });
    useStore.setState({
      agents: [AGENTS[1]!, { ...AGENTS[1]!, id: "chef-research", groupId: RESEARCH }],
    });
  }

  it("closes when it belongs to the crew being left", async () => {
    // The reported confusion, and why the name is the same on both: the rail
    // does not draw the row, so nothing on screen says which crew the pane
    // belongs to, and a namesake left open from the crew you came from reads as
    // this crew's, working while nobody here is.
    twoCrews();
    useStore.setState({ selected: "chef-research" });

    await useStore.getState().focusGroup(AGENTS[1]!.groupId);

    expect(useStore.getState().selected).toBe(ACTIVITY_CHANNEL);
    expect(useStore.getState().railGroup).toBe(AGENTS[1]!.groupId);
    // Landed on rather than merely pointed at: the feed the pane falls back to
    // is read here, and nowhere else.
    expect(useStore.getState().messages[ACTIVITY_CHANNEL]).toBeDefined();
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

  it("leaves the activity feed alone, because it belongs to no crew", async () => {
    twoCrews();
    useStore.setState({ selected: ACTIVITY_CHANNEL });

    await useStore.getState().focusGroup(RESEARCH);

    expect(useStore.getState().selected).toBe(ACTIVITY_CHANNEL);
    expect(useStore.getState().railGroup).toBe(RESEARCH);
  });
});

describe("one row at a time", () => {
  it("moves a row up past the one above it", async () => {
    reset();
    useStore.setState({
      agents: [
        { ...AGENTS[0]!, railOrder: 0 },
        { ...AGENTS[1]!, railOrder: 1 },
      ],
    });

    await useStore.getState().nudgeAgent("chef", -1);

    const { moveAgent } = (await import("./ipc")).api as unknown as {
      moveAgent: ReturnType<typeof vi.fn>;
    };
    expect(moveAgent).toHaveBeenCalledWith("chef", AGENTS[0]!.groupId, "manager");
  });

  it("orders from the arrangement and not from whoever is mid-turn", async () => {
    // The keyboard path has to agree with the drag: both edit the arrangement,
    // so neither may be measured against a rail with somebody lifted on top.
    reset();
    useStore.setState({
      agents: [
        { ...AGENTS[0]!, railOrder: 0 },
        { ...AGENTS[1]!, railOrder: 1 },
      ],
      activity: { chef: { state: "thinking" } },
    });

    await useStore.getState().nudgeAgent("chef", -1);

    const { moveAgent } = (await import("./ipc")).api as unknown as {
      moveAgent: ReturnType<typeof vi.fn>;
    };
    expect(moveAgent).toHaveBeenCalledWith("chef", AGENTS[0]!.groupId, "manager");
  });

  it("asks for nothing at the end of a section", async () => {
    reset();
    useStore.setState({
      agents: [
        { ...AGENTS[0]!, railOrder: 0 },
        { ...AGENTS[1]!, railOrder: 1 },
      ],
    });

    const { moveAgent } = (await import("./ipc")).api as unknown as {
      moveAgent: ReturnType<typeof vi.fn>;
    };
    moveAgent.mockClear();
    await useStore.getState().nudgeAgent("manager", -1);
    expect(moveAgent).not.toHaveBeenCalled();
  });
});

describe("activity", () => {
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
  it("drops what it is holding for the cleared channels, and the feed with them", async () => {
    // The bug this covers: clearing a group left the open transcript on screen
    // until the operator clicked away and back.
    reset({
      chef: [envelope()],
      manager: [envelope({ channelId: "manager" })],
      activity: [envelope()],
    });

    apply({ type: "channelsCleared", agents: ["chef"] });

    expect(useStore.getState().messages.chef).toBeUndefined();
    // The feed draws from every channel, so it is stale too.
    expect(useStore.getState().messages.activity).toBeUndefined();
    // A channel that was not cleared keeps what it had.
    expect(useStore.getState().messages.manager).toHaveLength(1);
  });

  it("reads the open channel again rather than leaving it blank", async () => {
    reset({ chef: [envelope()] });
    apply({ type: "channelsCleared", agents: ["chef"] });

    const { channelMessages } = (await import("./ipc")).api as unknown as {
      channelMessages: ReturnType<typeof vi.fn>;
    };
    // No third argument: a reload after a clear is not a jump to a message,
    // and asking to reach one that was just deleted would widen the window
    // for nothing.
    expect(channelMessages).toHaveBeenCalledWith("chef", 300, undefined);
  });
});

describe("openMessage", () => {
  it("asks for a window wide enough to hold the message", async () => {
    // The transcript is normally read as "the newest three hundred", and a
    // search hit from last month is not in it. Opening the channel without
    // saying which message would land the operator at the wrong end of it.
    reset({ chef: [] });
    await useStore.getState().openMessage("chef", "m-old");

    const { channelMessages } = (await import("./ipc")).api as unknown as {
      channelMessages: ReturnType<typeof vi.fn>;
    };
    expect(channelMessages).toHaveBeenCalledWith("chef", 300, "m-old");
  });

  it("marks the message before the read comes back", async () => {
    // The channel switches first so the operator sees where they are going,
    // and the mark has to be in place by the time the rows arrive or the
    // transcript has nothing to scroll to.
    reset({ chef: [] });
    const pending = useStore.getState().openMessage("chef", "m-old");

    expect(useStore.getState().selected).toBe("chef");
    expect(useStore.getState().focused).toBe("m-old");
    await pending;
  });

  it("drops the mark when the operator moves on by hand", async () => {
    // Otherwise clicking an agent in the rail re-runs the jump the next time
    // that channel draws.
    reset({ chef: [] });
    await useStore.getState().openMessage("chef", "m-old");
    await useStore.getState().select("manager");

    expect(useStore.getState().focused).toBeNull();
  });
});

describe("a coding job that could not run", () => {
  it("reaches the operator, not just the agent that asked", () => {
    // The afternoon this cost. An expired credential on the operator's own
    // machine stopped every coding job in the workspace, each agent was told in
    // its own channel, and the operator saw nothing at all.
    useStore.getState().applyEvent({
      type: "codingJobFailed",
      agentId: "a1",
      repository: "vision-ios",
      reason: "Provided authentication token is expired.",
    });

    const banner = useStore.getState().banner;
    expect(banner?.tone).toBe("error");
    expect(banner?.text).toContain("vision-ios");
    expect(banner?.text).toContain("expired");
  });

  it("holds what a running job is doing, and drops it when the job ends", () => {
    // The same discipline a turn's thinking has. The record of what a job did
    // is the message it delivers; this is what that looks like beforehand, so
    // keeping it after the fact would be a second copy that could disagree.
    useStore.getState().applyEvent({
      type: "codingJobStarted",
      agentId: "a1",
      repositoryId: "r1",
      repository: "vision-ios",
    });
    useStore.getState().applyEvent({
      type: "codingProgress",
      agentId: "a1",
      repositoryId: "r1",
      tool: "bash",
      detail: "swift test",
    });
    expect(useStore.getState().building.r1).toBe("a1");
    expect(useStore.getState().coding.a1).toEqual([{ tool: "bash", detail: "swift test" }]);

    useStore.getState().applyEvent({
      type: "codingJobFinished",
      agentId: "a1",
      repositoryId: "r1",
    });
    expect(useStore.getState().building.r1).toBeUndefined();
    expect(useStore.getState().coding.a1).toBeUndefined();
  });

  it("bounds the tail, because a long job runs hundreds of tools", () => {
    useStore.getState().applyEvent({
      type: "codingJobStarted",
      agentId: "a2",
      repositoryId: "r2",
      repository: "vision-ios-api",
    });
    for (let n = 0; n < 200; n++) {
      useStore.getState().applyEvent({
        type: "codingProgress",
        agentId: "a2",
        repositoryId: "r2",
        tool: "bash",
        detail: `step ${n}`,
      });
    }
    const held = useStore.getState().coding.a2 ?? [];
    expect(held.length).toBeLessThanOrEqual(40);
    // The newest end is the one kept: what it is doing now beats what it did
    // three hundred tools ago.
    expect(held.at(-1)?.detail).toBe("step 199");
  });

  it("names the repository, because an operator has more than one", () => {
    useStore.getState().applyEvent({
      type: "codingJobFailed",
      agentId: "a1",
      repository: "vision-ios-api",
      reason: "the `pi` coding harness is not installed",
    });
    expect(useStore.getState().banner?.text).toContain("vision-ios-api");
  });
});
