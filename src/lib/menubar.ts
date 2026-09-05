/**
 * What the menu bar is handed while the window is showing a box.
 *
 * The strip is drawn by Rust from this machine's runtime, and that is the
 * wrong runtime when the window is pointed at a box: the numbers in the
 * corner would be about a workspace nobody is looking at. The window already
 * holds the box's roster, activity, requests and spend, because it draws
 * them, so it hands the strip the same shape Rust reads locally and the strip
 * follows the window. Nothing here decides anything; it is one projection of
 * the store, and the test beside it is what keeps the projection honest.
 */

import type { State } from "./store";
import type { Presence, Tokens } from "./types";

/** The strip's view of the store, in the shape `menubar::Presence` reads. */
export function presenceOf(state: State): Presence {
  const roster: Presence["roster"] = {};
  for (const agent of state.agents) {
    if (agent.lifecycle === "terminated") continue;
    roster[agent.id] = { name: agent.name, crew: agent.groupId };
  }
  return {
    roster,
    crews: state.groups.map((group) => ({ id: group.id, name: group.name })),
    activity: state.activity,
    waiting: state.pending,
    stuck: state.stuck,
    session: state.sessionSpend,
    allTime: sum(Object.values(state.usage).filter((t): t is Tokens => t !== undefined)),
    running: Object.values(state.activeRun).filter((run) => run !== undefined).length,
  };
}

function sum(all: Tokens[]): Tokens {
  const total: Tokens = { prompt: 0, completion: 0, cost: null, calls: 0 };
  for (const tokens of all) {
    total.prompt += tokens.prompt;
    total.completion += tokens.completion;
    total.calls += tokens.calls;
    if (tokens.cost !== null) total.cost = (total.cost ?? 0) + tokens.cost;
  }
  return total;
}

/**
 * How long a burst of changes becomes one report.
 *
 * A streaming turn changes the store on every token, and the strip only has
 * a dot to move. Rust coalesces on its side too; this keeps the bridge quiet.
 */
export const FEED_COALESCE_MS = 200;

/**
 * Whether two presences would draw the same strip.
 *
 * Cheap enough to run on every store change and honest enough to skip the
 * report when nothing the strip draws has moved. A structural compare rather
 * than identity, because the store rebuilds objects it did not change.
 */
export function samePresence(a: Presence, b: Presence): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}
