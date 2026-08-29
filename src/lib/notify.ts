/**
 * Telling the operator something happened while they were not looking.
 *
 * The hard part is not raising a notification, it is refusing to. Guaca runs
 * unattended: routines fire on a schedule, an agent parks on a permission
 * request and waits ten minutes for an answer, a cascade settles long after the
 * message that started it. All of that is worth knowing about when you are
 * elsewhere and none of it is worth a badge when you are watching it happen.
 *
 * So a notification has to pass three gates, and they are not the same gate:
 *
 *   the kind      is this something you asked to hear about
 *   the moment    are you actually elsewhere, and elsewhere from what
 *   the burst     has this already been said in the last second
 *
 * The second is where the judgment is. "Away" is not one condition, because a
 * permission request and a finished conversation are not the same news:
 *
 *   attention   an agent is blocked on you, whether or not its turn is parked.
 *               Reaches you when the window is not in front of you, and also
 *               when it is but the request belongs to a channel you are not
 *               looking at. Nobody finds a parked turn by noticing a row change
 *               color three screens up the rail, and nobody finds an agent that
 *               gave up four turns ago by reading its channel.
 *   ambient     nothing was addressed to you and no channel is implied. A
 *               routine fires wherever it was pointed. Reaches you whenever you
 *               are away, with no channel to match against.
 *   completion  the end of something you started. Reaches you only when you are
 *               away AND it is the channel you were last looking at, because a
 *               busy runtime settles runs in channels you have never opened and
 *               one badge each would make the whole mechanism worth turning off.
 */

import type { NotifyKind, NotifyPrefs } from "./prefs";

export type NotifyClass = "attention" | "ambient" | "completion";

export function classOf(kind: NotifyKind): NotifyClass {
  switch (kind) {
    case "approval":
    // An escalation is the same class from the other end: nothing is parked on
    // it, so nothing lapses, and it has to reach the operator anyway because
    // the agent that raised it has already carried on without them.
    case "stuck":
      return "attention";
    case "routine":
      return "ambient";
    default:
      return "completion";
  }
}

/** Where the operator is, at the instant something happened. */
export interface Moment {
  /** The window is not the thing in front of the operator. */
  away: boolean;
  /**
   * The channel this concerns is the one on screen. Meaningless for `ambient`,
   * which has no channel to be about.
   */
  onScreen: boolean;
  /** Inside the quiet window that follows a launch. */
  quiet: boolean;
}

export function shouldNotify(kind: NotifyKind, prefs: NotifyPrefs, moment: Moment): boolean {
  if (!prefs.on || !prefs.kinds[kind]) return false;
  if (moment.quiet) return false;

  switch (classOf(kind)) {
    case "attention":
      return moment.away || !moment.onScreen;
    case "ambient":
      return moment.away;
    case "completion":
      return moment.away && moment.onScreen;
  }
}

/**
 * True when the window is not the thing the operator is looking at.
 *
 * `document.hidden` alone is not enough: it flips when the window is minimized
 * or fully covered, and a window sitting behind a browser on the same screen is
 * visible-but-unfocused, which is exactly the case worth notifying for.
 */
export function away(): boolean {
  if (document.hidden) return true;
  return typeof document.hasFocus === "function" && !document.hasFocus();
}

/**
 * How long after a launch nothing may interrupt.
 *
 * A routine whose slot passed while the app was closed reaches the scheduler
 * overdue and fires on the first tick, which is correct: the work is owed. But
 * it did not just happen, and launching after a weekend away should not
 * announce a weekend of schedule at once. The transcript and the rail show all
 * of it immediately; only the interruption waits.
 */
export const QUIET_MS = 4000;

let quietUntil = 0;

/** Opens the quiet window. Called once, when the first read of state lands. */
export function markQuiet(now = Date.now()): void {
  quietUntil = now + QUIET_MS;
}

export function quiet(now = Date.now()): boolean {
  return now < quietUntil;
}

/** Only tests need this: the window is process-wide and outlives a render. */
export function resetQuiet(): void {
  quietUntil = 0;
}

const BURST_MS = 1000;

const spoken = new Map<string, number>();

/**
 * True when this has already been said in the last second.
 *
 * Keyed on kind and channel, so two agents failing at once are two
 * notifications and one agent failing twice is one. The map prunes on every
 * call rather than on a timer, which is what keeps it from being a leak: an
 * entry older than the window is dropped by the next thing that asks.
 */
export function burst(key: string, now = Date.now()): boolean {
  for (const [seen, at] of spoken) {
    if (now - at >= BURST_MS) spoken.delete(seen);
  }

  if (spoken.has(key)) return true;

  spoken.set(key, now);
  return false;
}

/** Only tests need this, for the same reason as `resetQuiet`. */
export function resetBurst(): void {
  spoken.clear();
}
