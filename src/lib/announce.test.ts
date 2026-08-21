/**
 * What an interruption would say, decided one event at a time.
 *
 * `notify` owns whether the operator hears anything and has its own tests; this
 * decides what would be said and which channel it is about. Both ways of
 * getting that wrong are quiet. An announcement per event rather than per thing
 * that happened is a badge for every token of a reply, and the operator's only
 * answer to that is the master switch. And the channel is not decoration:
 * `notify` holds a completion back unless the operator was already looking at
 * that channel, so a kind labelled with the wrong one reaches nobody, or
 * reaches them for a run they have never opened.
 *
 * The one piece of state here, a run's channel, is module-level and outlives a
 * render, so what it forgets matters as much as what it learns.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

import { announcementFor, resetChannels } from "./announce";
import type { AgentId, Envelope, Part, RunId, UiEvent } from "./types";

const CHEF: AgentId = "agent-chef";
const SCRIBE: AgentId = "agent-scribe";
const RUN: RunId = "run-1";

const NAMES = new Map<AgentId, string>([
  [CHEF, "Chef"],
  [SCRIBE, "Scribe"],
]);

/** The lookup `App` hands in: a name when it has one, a stand-in when it does not. */
function nameOf(id: AgentId): string {
  return NAMES.get(id) ?? "An agent";
}

function envelope(over: Partial<Envelope> = {}): Envelope {
  return {
    id: "m1",
    runId: RUN,
    channelId: CHEF,
    from: { kind: "agent", id: CHEF },
    to: { kind: "human" },
    parts: [{ type: "text", text: "the listings are up to date" }],
    trust: "peer",
    hop: 1,
    expectsReply: false,
    intent: "work",
    cause: null,
    createdAt: 1,
    ...over,
  };
}

function appended(parts: Part[], over: Partial<Envelope> = {}): UiEvent {
  return { type: "messageAppended", message: envelope({ parts, ...over }) };
}

/** The event a run's channel is learned from, which is the only reason it is passed here. */
function opened(runId: RunId, channelId: AgentId): UiEvent {
  return {
    type: "streamStarted",
    messageId: `stream-${runId}`,
    channelId,
    agentId: channelId,
    runId,
    to: { kind: "human" },
  };
}

function ended(runId: RunId, stepsUsed: number): UiEvent {
  return { type: "runSettled", runId, stepsUsed };
}

beforeEach(() => {
  resetChannels();
});

describe("events nobody should be told about", () => {
  it("says nothing about the traffic a turn is made of", () => {
    // Every one of these arrives many times inside a single reply. A kind read
    // off the event stream rather than off the thing that happened is how a
    // finished conversation becomes forty notifications.
    const ignored: UiEvent[] = [
      { type: "agentsChanged" },
      { type: "streamDelta", messageId: "m1", channelId: CHEF, text: "the list" },
      { type: "reasoningDelta", messageId: "m1", text: "weighing it up" },
      { type: "streamEnded", messageId: "m1", channelId: CHEF },
      { type: "activityChanged", agentId: CHEF, activity: { state: "thinking" } },
      {
        type: "tokensUsed",
        agentId: CHEF,
        groupId: "group-1",
        runId: RUN,
        prompt: 900,
        completion: 40,
        cost: null,
      },
      { type: "approvalSettled", approvalId: "approval-1", state: "allow" },
      { type: "channelsCleared", agents: [CHEF, SCRIBE] },
    ];

    expect(ignored.map((event) => [event.type, announcementFor(event, nameOf)])).toEqual(
      ignored.map((event) => [event.type, null]),
    );
  });

  it("says nothing about a message that is only what an agent said", () => {
    // A reply in a channel is the app working, not news. The operator is told
    // when the conversation it belongs to finishes, once.
    expect(announcementFor(appended([{ type: "text", text: "done" }]), nameOf)).toBeNull();
  });

  it("says nothing about a message carrying the permission request itself", () => {
    // `approvalRequested` already announced this one. Announcing the part too
    // would put the same parked turn in front of the operator twice.
    const request = appended([
      {
        type: "approval",
        id: "approval-1",
        action: "actOnBehalf",
        summary: "Send the owner an email",
        detail: [{ label: "To", value: "owner@example.com" }],
      },
    ]);
    expect(announcementFor(request, nameOf)).toBeNull();
  });

  it("says nothing when the guard stopped a cascade", () => {
    // A limit doing its job is not a failure, and the operator usually caused
    // it by asking a crew to talk to itself. Treating it as one would announce
    // the machinery working correctly.
    const stopped = appended([
      { type: "notice", kind: "guardStop", text: "hop limit reached, nothing was sent" },
    ]);
    expect(announcementFor(stopped, nameOf)).toBeNull();
  });

  it("says nothing about a run that spent nothing", () => {
    // An envelope nobody read, or an agent that was already gone. Nothing ran,
    // so there is nothing the operator was waiting for.
    expect(announcementFor(opened(RUN, CHEF), nameOf)).toBeNull();
    expect(announcementFor(ended(RUN, 0), nameOf)).toBeNull();
  });
});

describe("the channel a run turns out to have been in", () => {
  it("is null for a run whose stream was never seen", () => {
    // Normal, not corrupt: runs are already in flight when the window
    // subscribes. The title has to be built without a channel rather than
    // interpolating one that is not there.
    const said = announcementFor(ended("run-elsewhere", 3), nameOf);
    expect(said?.channel).toBeNull();
    expect(said?.title).toBe("A conversation has finished");
    expect(said?.title).not.toMatch(/undefined/);
  });

  it("is forgotten even on the settle that announces nothing", () => {
    // The zero-step return is above the delete in an easy version of this
    // function, and then every quiet run leaks an entry for the life of the
    // process.
    announcementFor(opened(RUN, CHEF), nameOf);
    expect(announcementFor(ended(RUN, 0), nameOf)).toBeNull();
    expect(announcementFor(ended(RUN, 2), nameOf)?.channel).toBeNull();
  });

  it("is forgotten once the settle it was kept for has used it", () => {
    announcementFor(opened(RUN, CHEF), nameOf);
    expect(announcementFor(ended(RUN, 2), nameOf)?.channel).toBe(CHEF);
    expect(announcementFor(ended(RUN, 2), nameOf)?.channel).toBeNull();
  });

  it("belongs to one run and not to the process", () => {
    // Several runs are in flight at once by design, so a single "last channel
    // seen" would tag a settle with whichever run streamed most recently.
    announcementFor(opened("run-a", CHEF), nameOf);
    announcementFor(opened("run-b", SCRIBE), nameOf);
    expect(announcementFor(ended("run-b", 2), nameOf)?.channel).toBe(SCRIBE);
    expect(announcementFor(ended("run-a", 2), nameOf)?.channel).toBe(CHEF);
  });

  it("is learned from the stream that opened in it, which announces nothing itself", () => {
    expect(announcementFor(opened(RUN, SCRIBE), nameOf)).toBeNull();
    const said = announcementFor(ended(RUN, 3), nameOf);
    expect(said?.channel).toBe(SCRIBE);
    expect(said?.title).toBe("Scribe has finished");
  });
});

describe("the name the operator reads", () => {
  it("is asked for, never the agent's id", () => {
    const lookup = vi.fn(nameOf);
    const said = announcementFor(
      { type: "approvalRequested", approvalId: "approval-1", agentId: CHEF },
      lookup,
    );
    expect(lookup).toHaveBeenCalledWith(CHEF);
    expect(said?.title).toBe("Chef needs your permission");
    // A uuid in a notification tells the operator nothing about who is waiting.
    expect(said?.title).not.toContain(CHEF);
  });

  it("falls back to a stand-in for an agent nobody can name", () => {
    // An agent terminated between the event and the notification is not in the
    // store any more, and every one of these strings is user-facing.
    const gone: AgentId = "agent-gone";
    const said = [
      announcementFor(
        { type: "approvalRequested", approvalId: "approval-1", agentId: gone },
        nameOf,
      ),
      announcementFor(
        appended([{ type: "notice", kind: "upstreamError", text: "no route to host" }], {
          channelId: gone,
        }),
        nameOf,
      ),
    ];
    expect(said[0]?.title).toBe("An agent needs your permission");
    for (const one of said) {
      expect(one?.title).not.toMatch(/undefined/);
      expect(one?.body).not.toMatch(/undefined/);
    }
  });
});

describe("what each interruption says", () => {
  it("names the agent whose turn is parked, and the channel to answer in", () => {
    const said = announcementFor(
      { type: "approvalRequested", approvalId: "approval-1", agentId: SCRIBE },
      nameOf,
    );
    expect(said?.kind).toBe("approval");
    expect(said?.title).toBe("Scribe needs your permission");
    // `notify` lets an approval through when the operator is away *or* looking
    // at another channel, and it can only tell the second case from the channel.
    expect(said?.channel).toBe(SCRIBE);
  });

  it("keys a permission request on the agent rather than on the request", () => {
    // Two requests from one agent within the second are one interruption; the
    // burst check can only collapse them if the approval id is not in the key.
    const first = announcementFor(
      { type: "approvalRequested", approvalId: "approval-1", agentId: CHEF },
      nameOf,
    );
    const again = announcementFor(
      { type: "approvalRequested", approvalId: "approval-2", agentId: CHEF },
      nameOf,
    );
    const other = announcementFor(
      { type: "approvalRequested", approvalId: "approval-3", agentId: SCRIBE },
      nameOf,
    );
    expect(again?.key).toBe(first?.key);
    // And two agents parked at once are two, because nothing else says the
    // second one is waiting.
    expect(other?.key).not.toBe(first?.key);
  });

  it("announces a fired routine about no channel at all", () => {
    const said = announcementFor(
      appended(
        [
          {
            type: "routine",
            routineId: "routine-7",
            name: "Morning sweep",
            what: "check the listings",
          },
        ],
        { from: { kind: "system" }, trust: "system" },
      ),
      nameOf,
    );
    expect(said?.kind).toBe("routine");
    expect(said?.title).toBe("Morning sweep fired");
    expect(said?.body).toBe("check the listings");
    expect(said?.key).toBe("routine:routine-7");
    // Null even though the message has a channel. A routine goes where it was
    // pointed, almost never where the operator is looking, so carrying the
    // channel would let `notify` hold back the one kind that only ever happens
    // while nobody is watching.
    expect(said?.channel).toBeNull();
  });

  it("carries the upstream words when an agent could not reply", () => {
    const said = announcementFor(
      appended([{ type: "notice", kind: "upstreamError", text: "502 from the endpoint, gave up" }]),
      nameOf,
    );
    expect(said?.kind).toBe("failed");
    expect(said?.title).toBe("Chef could not reply");
    // A body of "something went wrong" sends the operator to the transcript to
    // find out which thing, and the endpoint is usually the answer.
    expect(said?.body).toBe("502 from the endpoint, gave up");
    expect(said?.channel).toBe(CHEF);
    expect(said?.key).toBe(`failed:${CHEF}`);
  });

  it("counts one model call in the singular", () => {
    announcementFor(opened(RUN, CHEF), nameOf);
    expect(announcementFor(ended(RUN, 1), nameOf)?.body).toBe("One model call.");
  });

  it("says how much a longer conversation took, and that it is over", () => {
    announcementFor(opened(RUN, CHEF), nameOf);
    const said = announcementFor(ended(RUN, 4), nameOf);
    expect(said?.kind).toBe("settled");
    expect(said?.body).toMatch(/^4 model calls\b/);
    expect(said?.key).toBe(`settled:${RUN}`);
  });
});
