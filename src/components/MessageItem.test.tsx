import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { Lookups, PeerSummary, Row } from "../lib/transcript";
import type { AgentCard, Envelope, Part } from "../lib/types";
import { MessageItem, ResultItem } from "./MessageItem";

const retryTurn = vi.fn<(agentId: string, messageId: string) => Promise<string>>(
  async () => "run-2",
);
vi.mock("../lib/ipc", () => ({
  openExternal: vi.fn(),
  api: { retryTurn: (agentId: string, messageId: string) => retryTurn(agentId, messageId) },
}));

const openRoutine = vi.fn<(id: string) => void>();
const showArtifacts = vi.fn<(open: { id: string | null } | null) => void>();
vi.mock("../lib/store", () => {
  const state = () => ({ openRoutine, setBanner: vi.fn(), showArtifacts });
  const useStore = (pick: (state: object) => unknown) => pick(state());
  useStore.getState = state;
  return { useStore };
});

function card(id: string, name: string): AgentCard {
  return {
    id,
    groupId: "00000000-0000-4000-8000-000000000001",
    sandboxId: null,
    browserId: null,
    hasComputer: false,
    hasBrowser: false,
    runsErrands: false,
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

const AGENTS = [card("manager", "Manager"), card("chef", "Chef")];

const lookups: Lookups = {
  byId: (id) => AGENTS.find((a) => a.id === id),
  byName: (name) => AGENTS.find((a) => a.name.toLowerCase() === name.toLowerCase()),
};

function envelope(overrides: Partial<Envelope>): Envelope {
  return {
    id: "m1",
    runId: "r1",
    channelId: "manager",
    from: { kind: "human" },
    to: { kind: "agent", id: "manager" },
    parts: [{ type: "text", text: "hello there" }],
    trust: "operator",
    hop: 0,
    expectsReply: true,
    intent: "courtesy",
    cause: null,
    createdAt: 1_700_000_000_000,
    ...overrides,
  };
}

function show(message: Envelope) {
  return render(<MessageItem message={message} lookups={lookups} continued={false} />);
}

describe("messages addressed to the operator", () => {
  it("renders what you said as a bubble, with no avatar", () => {
    const { container } = show(envelope({}));
    expect(screen.getByText("hello there")).toBeTruthy();
    expect(container.querySelector(".avatar")).toBeNull();
  });

  it("renders an agent's reply to you as a bubble, with an avatar", () => {
    const { container } = show(
      envelope({
        from: { kind: "agent", id: "manager" },
        to: { kind: "human" },
        parts: [{ type: "text", text: "on it" }],
      }),
    );
    expect(screen.getByText("on it")).toBeTruthy();
    expect(container.querySelector(".avatar")).toBeTruthy();
  });
});

describe("the running head", () => {
  const reply = envelope({
    from: { kind: "agent", id: "manager" },
    to: { kind: "human" },
    parts: [{ type: "text", text: "on it" }],
  });

  it("names the speaker on every turn, including in a channel of one agent", () => {
    render(<MessageItem message={reply} lookups={lookups} continued={false} />);
    expect(screen.getByText("Manager")).toBeTruthy();
    expect(document.querySelector(".msg__head .avatar")).toBeTruthy();
  });

  it("keeps your own message unmistakable in a bubble, and names you to a screen reader", () => {
    const { container } = render(
      <MessageItem message={envelope({})} lookups={lookups} continued={false} />,
    );
    expect(screen.getByRole("article", { name: /^You, / })).toBeTruthy();
    expect(container.querySelector(".msg[data-operator='true'] > .msg__bubble")).toBeTruthy();
    expect(container.querySelector(".avatar")).toBeNull();
    expect(container.querySelector(".msg__head")).toBeNull();
  });

  it("draws it once for a run of messages, on the first", () => {
    const { container } = render(<MessageItem message={reply} lookups={lookups} continued />);
    expect(screen.queryByText("Manager")).toBeNull();
    expect(container.querySelector(".msg--continued")).toBeTruthy();
  });

  it("carries the time on the head line, beside the name", () => {
    const { container } = render(
      <MessageItem message={reply} lookups={lookups} continued={false} />,
    );
    const at = container.querySelector("time.msg__at");
    expect(at).toBeTruthy();
    expect(at?.getAttribute("datetime")).toBe(new Date(reply.createdAt).toISOString());
    expect(at?.closest(".msg__head")?.textContent).toContain("Manager");
  });
});

describe("agent-to-agent traffic", () => {
  it("is a bubble here, because the only place it is drawn one by one is the pair's own thread", () => {
    show(
      envelope({
        channelId: "chef",
        from: { kind: "agent", id: "manager" },
        to: { kind: "agent", id: "chef" },
        hop: 2,
        parts: [{ type: "text", text: "a very long briefing document" }],
      }),
    );
    expect(screen.getByText("a very long briefing document")).toBeTruthy();
    expect(screen.getByText("Manager")).toBeTruthy();
  });
});

describe("an agent's own record of what it did", () => {
  const record = (part: Part) =>
    envelope({
      from: { kind: "agent", id: "manager" },
      to: { kind: "system" },
      trust: "system",
      parts: [part],
    });

  it("keeps a directory lookup quiet and unclickable", () => {
    show(
      record({
        type: "toolCall",
        name: "directory",
        arguments: {},
        outcome: { status: "ok", summary: "2 agent(s): Chef, Scribe" },
      }),
    );
    expect(screen.getByText("Checked who is available")).toBeTruthy();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("draws a call that wrote a kept page as a card that opens it", () => {
    show(
      record({
        type: "toolCall",
        name: "artifact",
        arguments: { action: "create", title: "Pipeline by stage", page: "<p/>" },
        outcome: { status: "ok", summary: "Created" },
        artifact: { id: "artifact-1", version: 1, title: "Pipeline by stage" },
      }),
    );
    expect(screen.getByText("New artifact")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /Pipeline by stage/ }));
    expect(showArtifacts).toHaveBeenCalledWith({ id: "artifact-1" });
  });

  it("draws a refused artifact call as a chip, with no card to open", () => {
    show(
      record({
        type: "toolCall",
        name: "artifact",
        arguments: { action: "update", id: "artifact-1", page: "<p/>" },
        outcome: { status: "refused", reason: "`update` needs a `note`" },
      }),
    );
    expect(screen.getByText(/Tried to update an artifact/)).toBeTruthy();
    expect(screen.queryByText("New artifact")).toBeNull();
  });

  it("does not draw a memory update as a message to nobody", () => {
    show(
      record({
        type: "toolCall",
        name: "update_memory",
        arguments: { content: "Smith handles verification." },
        outcome: { status: "ok", summary: "Memory saved (28 characters)." },
      }),
    );
    expect(screen.queryByText(/no one/)).toBeNull();
    expect(screen.getByText("Updated its memory")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /Updated its memory/ }));
    expect(screen.getByText("Smith handles verification.")).toBeTruthy();
  });

  it("opens a memory rewrite against the version it replaced", () => {
    show(
      record({
        type: "toolCall",
        name: "update_memory",
        arguments: { content: "Smith handles verification.\nJones signs off." },
        outcome: { status: "ok", summary: "Memory saved (44 characters)." },
        replaced: "Smith handles verification.\nPatel signs off.",
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: /Updated its memory/ }));

    const kind = (text: string) =>
      screen.getByText(text).closest("[data-kind]")?.getAttribute("data-kind");
    expect(kind("Smith handles verification.")).toBe("same");
    expect(kind("Patel signs off.")).toBe("removed");
    expect(kind("Jones signs off.")).toBe("added");
    const marks = [...document.querySelectorAll(".diff__mark")].map((node) => node.textContent);
    expect(marks).toEqual([" ", "-", "+"]);
    expect(screen.getByText("1 line added, 1 removed")).toBeTruthy();
    expect(screen.queryByText(/44 characters/)).toBeNull();
  });

  it("opens a memory that was cleared, which has nothing to show but the loss", () => {
    show(
      record({
        type: "toolCall",
        name: "update_memory",
        arguments: { content: "" },
        outcome: { status: "ok", summary: "Memory cleared." },
        replaced: "Smith handles verification.",
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: /Updated its memory/ }));
    const line = screen.getByText("Smith handles verification.").closest("[data-kind]");
    expect(line?.getAttribute("data-kind")).toBe("removed");
  });

  it("draws a memory recorded before any of this as what it wrote, and no diff", () => {
    show(
      record({
        type: "toolCall",
        name: "update_memory",
        arguments: { content: "Smith handles verification.\nJones signs off." },
        outcome: { status: "ok", summary: "Memory saved (44 characters)." },
      }),
    );
    fireEvent.click(screen.getByRole("button", { name: /Updated its memory/ }));
    expect(document.querySelector(".diff")).toBeNull();
    expect(screen.getByText(/Smith handles verification/)).toBeTruthy();
  });

  it("names an unrecognized tool rather than guessing it was a send", () => {
    show(
      record({
        type: "toolCall",
        name: "run_code",
        arguments: { source: "print(1)" },
        outcome: { status: "ok", summary: "exit 0" },
      }),
    );
    expect(screen.queryByText(/no one/)).toBeNull();
    expect(screen.getByText("Used run_code")).toBeTruthy();
  });

  it("says why a tool call failed, not just that it happened", () => {
    show(
      record({
        type: "toolCall",
        name: "send_message",
        arguments: { text: "hello?" },
        outcome: { status: "refused", reason: "Refused: name a recipient." },
      }),
    );
    expect(screen.getByText(/name a recipient/)).toBeTruthy();
  });

  it("surfaces a guard stop as a centered notice", () => {
    show(record({ type: "notice", kind: "guardStop", text: "hop limit (8) reached" }));
    expect(screen.getByText("hop limit (8) reached")).toBeTruthy();
  });

  it("draws text the turn trailed as an aside, not as a message to the operator", () => {
    const { container } = show(
      record({ type: "text", text: "Replied to Manager confirming how I work." }),
    );
    expect(screen.getByText("Replied to Manager confirming how I work.")).toBeTruthy();
    expect(container.querySelector(".aside")).toBeTruthy();
    expect(container.querySelector(".msg")).toBeNull();
  });
});

describe("a failed turn", () => {
  function failure(cause: string | null): Envelope {
    return envelope({
      id: "notice-1",
      from: { kind: "system" },
      to: { kind: "agent", id: "manager" },
      trust: "system",
      expectsReply: false,
      cause,
      parts: [
        {
          type: "notice",
          kind: "upstreamError",
          text: "Manager could not reply: could not reach the inference endpoint",
        },
      ],
    });
  }

  it("offers to send the message again, and says which", () => {
    retryTurn.mockClear();
    render(<MessageItem message={failure("m-original")} lookups={lookups} continued={false} />);

    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(retryTurn).toHaveBeenCalledWith("manager", "m-original");
    expect((screen.getByRole("button", { name: "Sent again" }) as HTMLButtonElement).disabled).toBe(
      true,
    );
  });

  it("requires an explicit retry for work interrupted by a backend restart", () => {
    retryTurn.mockClear();
    const interrupted = failure("m-original");
    interrupted.parts = [
      {
        type: "notice",
        kind: "interrupted",
        text: "The backend restarted. Review previous actions before retrying.",
      },
    ];
    render(<MessageItem message={interrupted} lookups={lookups} continued={false} />);
    expect(retryTurn).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(retryTurn).toHaveBeenCalledWith("manager", "m-original");
  });

  it("offers nothing when there is nothing to send again", () => {
    render(<MessageItem message={failure(null)} lookups={lookups} continued={false} />);
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
  });

  it("does not offer a retry for a limit that would be hit again", () => {
    const stopped = envelope({
      from: { kind: "system" },
      to: { kind: "agent", id: "manager" },
      cause: "m-original",
      parts: [{ type: "notice", kind: "guardStop", text: "this conversation used its budget" }],
    });
    render(<MessageItem message={stopped} lookups={lookups} continued={false} />);
    expect(screen.queryByRole("button", { name: "Try again" })).toBeNull();
  });
});

describe("a command that used a credential", () => {
  it("says so in the transcript, by name, with no value anywhere", () => {
    const used = envelope({
      from: { kind: "agent", id: "manager" },
      to: { kind: "system" },
      parts: [
        {
          type: "toolCall",
          name: "run_command",
          arguments: { command: 'curl -H "Authorization: Bearer $MISTRAL_API_KEY" ...' },
          outcome: {
            status: "ok",
            summary: "used Mistral ($MISTRAL_API_KEY) · exit 0, 812 bytes out",
          },
        },
      ],
    });
    const { container } = render(
      <MessageItem message={used} lookups={lookups} continued={false} />,
    );

    expect(screen.getByText("Mistral ($MISTRAL_API_KEY)")).toBeTruthy();
    expect(container.textContent).not.toContain("exit 0, 812 bytes out");
    expect(container.textContent).not.toContain("curl -H");

    fireEvent.click(screen.getByRole("button", { name: /Ran a command/ }));
    expect(container.textContent).toContain("exit 0, 812 bytes out");
    expect(screen.getByText(/Authorization: Bearer \$MISTRAL_API_KEY/)).toBeTruthy();
  });
});

describe("a turn that asked the operator something", () => {
  it("says what it did, and keeps the name a model calls it by one click away", () => {
    const asking = envelope({
      from: { kind: "agent", id: "manager" },
      to: { kind: "system" },
      trust: "system",
      parts: [
        {
          type: "toolCall",
          name: "ask_operator",
          arguments: { question: "Ship it on Friday?" },
          outcome: { status: "ok", summary: "answered" },
        },
      ],
    });
    const { container } = show(asking);
    const note = screen.getByRole("button", { name: /Asked you/ });
    expect(note.textContent).not.toContain("ask_operator");
    expect(container.querySelector(".footnote__label")?.getAttribute("title")).toBe("ask_operator");

    fireEvent.click(note);
    const row = container.querySelector(".trail__step-title");
    expect(row?.textContent).toBe("Asked you");
    expect(row?.getAttribute("title")).toBe("ask_operator");
    expect(screen.getByText("Ship it on Friday?")).toBeTruthy();
  });
});

describe("a turn that used its computer two dozen times", () => {
  function browsing(): Envelope {
    return envelope({
      from: { kind: "agent", id: "manager" },
      to: { kind: "system" },
      trust: "system",
      parts: [
        {
          type: "toolCall",
          name: "browse",
          arguments: { action: "open", url: "https://cnn.com" },
          outcome: { status: "ok", summary: "open in the browser" },
        },
        {
          type: "toolCall",
          name: "browse",
          arguments: { action: "read" },
          outcome: { status: "ok", summary: "read in the browser" },
        },
        {
          type: "toolCall",
          name: "browse",
          arguments: { action: "click", id: 12 },
          outcome: { status: "ok", summary: "click in the browser" },
        },
      ],
    });
  }

  it("draws one line for the run of them, naming where it went", () => {
    const { container } = render(
      <MessageItem message={browsing()} lookups={lookups} continued={false} />,
    );
    expect(screen.getByRole("button", { name: /3 steps on cnn.com/ })).toBeTruthy();
    expect(container.querySelectorAll(".footnote__call")).toHaveLength(1);
  });

  it("opens the calls behind it, in order, and closes again", () => {
    render(<MessageItem message={browsing()} lookups={lookups} continued={false} />);
    const chip = screen.getByRole("button", { name: /3 steps on cnn.com/ });
    expect(chip.getAttribute("aria-expanded")).toBe("false");

    fireEvent.click(chip);
    expect(chip.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("Opened cnn.com")).toBeTruthy();
    expect(screen.getByText("Read the page")).toBeTruthy();
    expect(screen.getByText("Clicked on the page")).toBeTruthy();

    fireEvent.click(chip);
    expect(screen.queryByText("Read the page")).toBeNull();
  });
});

describe("a refusal the runtime wrote for a model", () => {
  function refused(): Envelope {
    return envelope({
      from: { kind: "agent", id: "manager" },
      to: { kind: "system" },
      trust: "system",
      parts: [
        {
          type: "toolCall",
          name: "send_message",
          arguments: { text: "on it" },
          outcome: {
            status: "failed",
            error: "send_message needs a non-empty `to` list of agent names",
          },
        },
      ],
    });
  }

  it("says it in words for the operator, and keeps the runtime's behind one click", () => {
    const { container } = render(
      <MessageItem message={refused()} lookups={lookups} continued={false} />,
    );
    const chip = screen.getByRole("button", { name: /send a message to nobody/ });
    expect(container.querySelector(".footnote__gist")?.textContent).toBe(
      "Nothing was sent: the call named nobody to send to",
    );
    expect(container.textContent).not.toContain("`to`");

    fireEvent.click(chip);
    expect(container.querySelector(".trail__why")?.textContent).toBe(
      "Nothing was sent: the call named nobody to send to.",
    );
    const detail = container.querySelector<HTMLDetailsElement>(".trail__told");
    expect(detail?.open).toBe(false);
    expect(detail?.textContent).toContain("What the agent was told");
    expect(detail?.textContent).toContain("`to`");
  });
});

describe("redrawing a transcript", () => {
  it("does not draw an entry again when nothing about it changed", () => {
    const message = envelope({ parts: [{ type: "text", text: "the answer is 42" }] });
    const view = render(<MessageItem message={message} lookups={lookups} continued={false} />);
    const drawn = screen.getByText("the answer is 42");

    view.rerender(<MessageItem message={message} lookups={lookups} continued={false} />);

    expect(screen.getByText("the answer is 42")).toBe(drawn);
  });

  it("still redraws when the message itself changes", () => {
    const view = render(
      <MessageItem
        message={envelope({ parts: [{ type: "text", text: "first" }] })}
        lookups={lookups}
        continued={false}
      />,
    );
    view.rerender(
      <MessageItem
        message={envelope({ parts: [{ type: "text", text: "second" }] })}
        lookups={lookups}
        continued={false}
      />,
    );

    expect(screen.getByText("second")).toBeTruthy();
    expect(screen.queryByText("first")).toBeNull();
  });
});

describe("a decision the operator answered in For you", () => {
  const request = {
    question: "Which address goes in the footer?",
    context: "FTC guidance asks for a valid postal address.",
    recommendation: "",
    options: [],
    source: "FTC CAN-SPAM compliance guide",
  };
  const answered = (part: Partial<Extract<Part, { type: "decision" }>> = {}) =>
    envelope({
      from: { kind: "system" },
      trust: "system",
      intent: "work",
      parts: [
        {
          type: "decision",
          id: "6bd01bb1-068b-433e-a520-fb87b0f85f5f",
          request,
          answer: "Leave it out.\nShow me the copy before you send anything.",
          resumed: false,
          ...part,
        },
      ],
    });

  it("is the operator's reply under the question it answers, not Guaca's brief", () => {
    const { container } = show(answered());

    expect(container.querySelector('[data-operator="true"]')).not.toBeNull();
    expect(screen.getByText("Answered in For you")).toBeTruthy();
    expect(screen.getByText(request.question)).toBeTruthy();
    expect(screen.getByText(/Leave it out\./)).toBeTruthy();
    expect(screen.queryByText("Guaca")).toBeNull();
    expect(container.textContent).not.toMatch(/answered decision|"answer"|6bd01bb1/);
  });

  it("keeps what the agent asked with behind the question", () => {
    const { container } = show(answered());
    const asked = container.querySelector("details");

    expect(asked?.open).toBe(false);
    expect(asked?.querySelector("summary")?.textContent).toBe(request.question);
    expect(asked?.textContent).toContain(request.context);
    expect(asked?.textContent).toContain(request.source);
    expect(asked?.textContent).not.toContain("Recommended");
  });

  it("draws the agent's words as text, and a bare question opens nothing", () => {
    const { container } = show(
      answered({
        request: { ...request, question: "Ship **now**?", context: "", source: "" },
        answer: "[yes](https://example.com)",
      }),
    );

    expect(screen.getByText("Ship **now**?")).toBeTruthy();
    expect(screen.getByText("[yes](https://example.com)")).toBeTruthy();
    expect(container.querySelector("a, strong, details")).toBeNull();
  });

  it("says so when it resumed follow-through", () => {
    show(answered({ resumed: true }));
    expect(screen.getByText("Resumed in For you")).toBeTruthy();
  });
});

describe("a routine coming due", () => {
  const fired = (parts?: Part[]) =>
    envelope({
      from: { kind: "system" },
      to: { kind: "agent", id: "manager" },
      intent: "work",
      parts: parts ?? [
        {
          type: "routine",
          routineId: "rt1",
          name: "Listings sweep",
          what: "Check the listings. America/New_York. Post one copy and say what changed.",
          payload: null,
        },
      ],
    });

  it("is one line naming the routine, not the instruction as dialogue", () => {
    show(fired());

    expect(screen.getByText("Listings sweep")).toBeTruthy();
    expect(screen.getByText("routine ran")).toBeTruthy();
    expect(screen.queryByText(/America\/New_York/)).toBeNull();
    expect(screen.queryByText("Guaca")).toBeNull();
  });

  it("titles an unnamed routine by what it says, like the schedule panel does", () => {
    show(
      fired([
        {
          type: "routine",
          routineId: "rt1",
          name: "",
          what: "Check the listings. Then post.",
          payload: null,
        },
      ]),
    );
    expect(screen.getByText("Check the listings")).toBeTruthy();
  });

  it("opens the routine it names", () => {
    show(fired());
    fireEvent.click(screen.getByRole("button"));
    expect(openRoutine).toHaveBeenCalledWith("rt1");
  });
});

describe("an overnight tidy", () => {
  const tidy = envelope({
    from: { kind: "system" },
    to: { kind: "agent", id: "manager" },
    intent: "work",
    expectsReply: false,
    parts: [
      {
        type: "tidy",
        reported: ["- 2h ago: Booked the venue."],
        notes: ["- 1h ago — Waiting on the caterer's quote."],
      },
    ],
  });

  it("is one line saying it ran, and nothing it handed the agent", () => {
    show(tidy);
    expect(screen.getByText("Overnight memory tidy")).toBeTruthy();
    expect(screen.getByText("ran")).toBeTruthy();
    expect(screen.queryByText(/Booked the venue/)).toBeNull();
    expect(screen.queryByText(/caterer/)).toBeNull();
    expect(screen.queryByText(/Time to tidy/)).toBeNull();
    expect(screen.queryByText("Guaca")).toBeNull();
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("tells the time the way a routine's line does", () => {
    const routine = envelope({
      from: { kind: "system" },
      to: { kind: "agent", id: "manager" },
      intent: "work",
      parts: [{ type: "routine", routineId: "rt1", name: "Sweep", what: "Sweep.", payload: null }],
    });
    const { container: ran } = show(tidy);
    const { container: fired } = show(routine);
    const time = (container: HTMLElement) => container.querySelector(".wire__meta")?.textContent;
    expect(time(ran)).toBeTruthy();
    expect(time(ran)).toBe(time(fired));
  });
});

describe("your brief once it is sent", () => {
  const brief = "Compare buying and leasing the next car. ".repeat(20);

  function overflowing(by: number) {
    vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockReturnValue(100 + by);
    vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(100);
  }

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("offers nothing to open when it fits in its few lines", () => {
    overflowing(0);
    render(
      <MessageItem
        message={envelope({ parts: [{ type: "text", text: "Hi." }] })}
        lookups={lookups}
        continued={false}
      />,
    );
    expect(screen.queryByRole("button", { name: "Show all" })).toBeNull();
  });

  it("folds to a few lines and opens on request when it is longer", () => {
    overflowing(80);
    const { container } = render(
      <MessageItem
        message={envelope({ parts: [{ type: "text", text: brief }] })}
        lookups={lookups}
        continued={false}
      />,
    );
    const folded = container.querySelector(".msg__brief");
    expect(folded?.getAttribute("data-folded")).toBe("true");
    const more = screen.getByRole("button", { name: "Show all" });
    expect(more.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(more);
    expect(folded?.hasAttribute("data-folded")).toBe(false);
    expect(screen.getByRole("button", { name: "Show less" }).getAttribute("aria-expanded")).toBe(
      "true",
    );
    expect(folded?.textContent).toBe(brief.trim());
  });

  it("keeps brief text readable while links wait for expansion", () => {
    overflowing(80);
    render(
      <MessageItem
        message={envelope({
          parts: [
            {
              type: "text",
              text: "[Read the research](https://example.com)\n\n    A very long command",
            },
          ],
        })}
        lookups={lookups}
        continued={false}
      />,
    );
    expect(screen.getByText("Read the research")).toBeTruthy();
    expect(screen.queryByRole("link", { name: "Read the research" })).toBeNull();
    const code = screen.getByText("A very long command").closest("pre");
    expect(code?.getAttribute("tabindex")).toBe("-1");
    fireEvent.click(screen.getByRole("button", { name: "Show all" }));
    expect(screen.getByRole("link", { name: "Read the research" })).toBeTruthy();
    expect(screen.getByText("A very long command").closest("pre")?.hasAttribute("tabindex")).toBe(
      false,
    );
  });

  it("never folds what an agent wrote", () => {
    overflowing(80);
    render(
      <MessageItem
        message={envelope({
          from: { kind: "agent", id: "manager" },
          to: { kind: "human" },
          parts: [{ type: "text", text: brief }],
        })}
        lookups={lookups}
        continued={false}
      />,
    );
    expect(screen.queryByRole("button", { name: "Show all" })).toBeNull();
  });
});

describe("the result a conversation ends in", () => {
  const answer = "Lease, if you replace the car within **three years**.";
  const message = envelope({
    id: "m9",
    from: { kind: "agent", id: "manager" },
    to: { kind: "human" },
    parts: [{ type: "text", text: answer }],
  });
  const chef: PeerSummary = {
    peer: { id: "chef", name: "Chef", color: "#c7d96b", avatar: "plain" },
    agentId: "chef",
    sent: 1,
    received: 1,
    gist: null,
    asked: "Which is cheaper?",
    askedAt: 1_000,
    answer: "Leasing.",
    answeredAt: 2_000,
  };
  const handoff: Extract<Row, { kind: "peers" }> = {
    kind: "peers",
    key: "peers:1",
    runId: "r1",
    peers: [chef],
    folded: [],
    at: 1_000,
  };

  function draw(row: Extract<Row, { kind: "result" }>, onToggle = vi.fn()) {
    return render(
      <ResultItem
        row={row}
        lookups={lookups}
        channel="manager"
        open={new Set()}
        onToggle={onToggle}
        onOpen={vi.fn()}
      />,
    );
  }

  it("is its own card, named for the agent that produced it", () => {
    const { container } = draw({ kind: "result", key: "m9", message, handoffs: [] });
    const result = screen.getByRole("article", { name: /^Result from Manager, / });
    expect(result.querySelector(".result")).toBeTruthy();
    expect(result.querySelector(".msg__name")?.textContent).toBe("Manager");
    expect(container.querySelector(".msg__body")).toBeNull();
  });

  it("keeps the agent's words exactly as written", () => {
    draw({ kind: "result", key: "m9", message, handoffs: [] });
    const result = screen.getByRole("article", { name: /^Result from/ });
    expect(result.querySelector(".result__body")?.textContent).toBe(
      "Lease, if you replace the car within three years.",
    );
    expect(result.querySelector(".result__body strong")?.textContent).toBe("three years");
  });

  it("carries the hand-off above it as a record that says who answered, and opens it", () => {
    const onToggle = vi.fn();
    const { container } = draw(
      { kind: "result", key: "m9", message, handoffs: [handoff] },
      onToggle,
    );
    const record = screen.getByRole("button", { name: /Asked Chef, answered/ });
    const card = container.querySelector(".result");
    expect(
      record.compareDocumentPosition(card as Node) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(record.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(record);
    expect(onToggle).toHaveBeenCalledTimes(1);
  });
});
