/**
 * Turning runtime events into the four things worth interrupting someone for.
 *
 * Kept apart from `notify`, which decides *whether* to interrupt, and from
 * `App`, which is a view. This decides *what* the interruption would say, and
 * it is a pure function of one event so it can be argued with in a test rather
 * than in a running window.
 *
 * Three of the four kinds are read out of the event stream directly. The fourth
 * is not: `runSettled` says which run ended and nothing about where it
 * happened, and "only tell me about the channel I was looking at" needs a
 * channel. So the one piece of state here is a run's channel, learned from the
 * `streamStarted` that opened in it and forgotten when the run ends. It is the
 * smallest thing that answers the question, and it cleans itself up on the same
 * event it is needed for.
 */

import type { NotifyKind } from "./prefs";
import type { AgentId, RunId, UiEvent } from "./types";

export interface Announcement {
  kind: NotifyKind;
  title: string;
  body: string;
  /**
   * The channel this is about, or null when it is about none. Null means the
   * "was I looking at it" question does not apply, which is only true of the
   * ambient kinds.
   */
  channel: AgentId | null;
  /** What the one-second burst check is keyed on. */
  key: string;
}

/**
 * Where each live run started talking.
 *
 * The FIRST placeholder, not the most recent, and that is the whole point. A
 * run spans as many channels as it reached, so the last agent to speak in a
 * cascade is usually one the operator has never opened; judging "was I looking
 * at this" against that channel answers a question nobody asked. The channel a
 * run opened in is the one the operator was in when they set it off.
 *
 * Bounded by the runs in flight rather than by the runs there have ever been,
 * because every entry is removed by the `runSettled` that made it interesting.
 * A run that never settles would leak one entry, and that is a defect the
 * trajectory suite fails on long before it is a memory problem here.
 */
const channels = new Map<RunId, AgentId>();

/** Only tests need this: the map outlives a render. */
export function resetChannels(): void {
  channels.clear();
}

export function announcementFor(
  event: UiEvent,
  nameOf: (id: AgentId) => string,
): Announcement | null {
  switch (event.type) {
    case "streamStarted":
      // Not an announcement. A stream is how a run says where it is, and the
      // only reason this function sees the event at all. Only the first one
      // counts: see the note on `channels`.
      if (!channels.has(event.runId)) channels.set(event.runId, event.channelId);
      return null;

    case "approvalRequested":
      return {
        kind: "approval",
        title: `${nameOf(event.agentId)} needs your permission`,
        body: "Its turn is parked until you answer, and gives up after ten minutes.",
        channel: event.agentId,
        key: `approval:${event.agentId}`,
      };

    case "runSettled": {
      const channel = channels.get(event.runId) ?? null;
      channels.delete(event.runId);
      // A run that spent nothing did not do anything worth announcing: an
      // envelope nobody read, an agent that was already gone. The operator was
      // not waiting for it.
      if (event.stepsUsed === 0) return null;
      return {
        kind: "settled",
        title: channel ? `${nameOf(channel)} has finished` : "A conversation has finished",
        body:
          event.stepsUsed === 1
            ? "One model call."
            : `${event.stepsUsed} model calls, and everyone it reached has gone quiet.`,
        channel,
        key: `settled:${event.runId}`,
      };
    }

    case "messageAppended": {
      const message = event.message;

      for (const part of message.parts) {
        if (part.type === "routine") {
          return {
            kind: "routine",
            title: `${part.name} fired`,
            body: part.what,
            // Deliberately null even though the message has a channel: a
            // routine goes where it was pointed, which is almost never where
            // the operator is looking, and holding it back for that reason
            // would be holding back the one kind that only happens while
            // nobody is watching.
            channel: null,
            key: `routine:${part.routineId}`,
          };
        }

        if (part.type === "notice" && part.kind === "upstreamError") {
          return {
            kind: "failed",
            title: `${nameOf(message.channelId)} could not reply`,
            body: part.text,
            channel: message.channelId,
            key: `failed:${message.channelId}`,
          };
        }
      }

      return null;
    }

    default:
      return null;
  }
}
