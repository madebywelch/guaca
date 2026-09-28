/**
 * What a row says an agent is doing, in the words beside its name.
 *
 * One wording, drawn in two places: the rail, and the panel under the menu bar
 * icon. The two are read for the same reason, which is somebody scanning a
 * list of faces for the one that needs them, and a panel that said "thinking"
 * where the rail said "typing" would be two stories about one agent.
 */

import { relativeTime } from "./time";
import { type LiveCall, machineInUse } from "./trail";
import type { Activity, AgentId, Escalation } from "./types";

/** The line, and which of the rail's states it is drawn as. */
export interface Doing {
  text: string;
  kind: "thinking" | "queued" | "asking" | "paused" | undefined;
}

/** Everything the line is read from, as the store holds it. */
export interface DoingInputs {
  activity: Record<AgentId, Activity>;
  stuck: Escalation[];
  building: Record<AgentId, string>;
  trail: Record<AgentId, LiveCall[] | undefined>;
  lastActive: Record<AgentId, number>;
}

export function doing(id: AgentId, inputs: DoingInputs, now: number): Doing {
  const state = inputs.activity[id];

  // Above everything but a parked turn, and above it in the same voice. Both
  // of the states a person is the fix for outrank every state that will
  // resolve itself, because this column is what somebody scans when they have
  // noticed that nothing is moving: an agent stuck since Tuesday that happens
  // to be typing right now must not read as an agent that is fine.
  //
  // The age is the label. "Stuck" is a state and "stuck 2d" is a decision,
  // and the second one is the whole reason this is on the row rather than in
  // a channel somebody has to open.
  if (state?.state !== "awaitingApproval") {
    const open = inputs.stuck.find((one) => one.agentId === id);
    if (open) {
      return { text: `stuck ${relativeTime(open.raisedAt, now)}`, kind: "asking" };
    }
  }

  // Before the turn states, because a coding job outlives the turn that
  // started it: the agent goes idle the moment `code` returns and stays that
  // way while a coding agent works for it for twenty minutes. Read off
  // `building` rather than off `Activity` for the same reason: `Activity` is
  // cleared when a turn ends, and this is not a turn.
  if (inputs.building[id] !== undefined) {
    return { text: "writing code", kind: "thinking" };
  }

  switch (state?.state) {
    case "thinking":
      // The machine over the model. "typing" is what a model does, and an
      // agent driving its computer is doing that too; the row says the
      // thing the operator can go and watch.
      switch (machineInUse(inputs.trail[id])) {
        case "computer":
          return { text: "on its computer", kind: "thinking" };
        case "browser":
          return { text: "in its browser", kind: "thinking" };
        default:
          return { text: "typing", kind: "thinking" };
      }
    case "queued":
      return { text: `${state.depth} queued`, kind: "queued" };
    // The one state the operator is the fix for. It reads as an instruction
    // rather than a status because the agent is parked until they act, and
    // the request itself is in a channel they may not have open.
    case "awaitingApproval":
      return { text: "needs you", kind: "asking" };
    case "paused":
      return { text: "paused", kind: "paused" };
    default: {
      // Falling back to a last-active time keeps the column occupied. An
      // empty slot the instant an agent stops typing reads as the row
      // breaking rather than the agent finishing.
      const at = inputs.lastActive[id];
      return { text: at ? relativeTime(at, now) : "", kind: undefined };
    }
  }
}
