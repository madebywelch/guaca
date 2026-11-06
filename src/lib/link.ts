/**
 * The line to the host: whether this page is hearing from it, and when it last
 * did.
 *
 * The window is a client and the host is somewhere else, which makes the worst
 * thing it can do look like it still knows what is going on: the rail says who
 * is working, the transcript ends where it ended, and nothing says that is a
 * picture of a minute ago. A call that fails says so on the banner, but only
 * once somebody asks for something; the socket closing is the one moment the
 * host's absence is knowable without asking. `runtime.ts` raises the mark on
 * that close and clears it on the next open, and three surfaces draw it: the
 * rail's ground, the transcript, and the status bar that names the host.
 *
 * What is here is the other half: when a host that stops answering is not
 * news. It cannot be a flag that an update clears, because a host that never
 * comes back clears nothing, and a page that never says it lost its host is the
 * failure this exists to end. So it is a window, held open by every read that
 * says an update is still underway.
 */

/** Whether this page is hearing from its host. */
export type Link = "up" | "down";

/**
 * How long a host that stops answering may be an update at work.
 *
 * A host an update restarts goes away on purpose, and `docs/UPDATES.md` is
 * explicit that not answering during an update is drawn as the host restarting
 * rather than as a failure: the panel that started it says so for as long as it
 * takes. This is that rule in the place the rest of the window can read it, so
 * the banner does not argue with the panel.
 *
 * A window rather than a flag because the last thing an update does before its
 * host goes quiet cannot be read from a host that is quiet. Long enough for a
 * container to stop, copy its volume and pass its health check — the backup is
 * a copy nobody measured, which is why `lib/updating.ts` counts steps and never
 * time — and short enough that a host still gone afterwards is worth saying.
 */
const RESTART_QUIET_MS = 180_000;

let quietUntil = 0;

/**
 * Records that the host is being restarted on purpose, from a read that said an
 * update is underway.
 *
 * Called on every such read rather than once at the click, so a restart that
 * runs longer than the window keeps moving with it.
 */
export function expectRestart(): void {
  quietUntil = Date.now() + RESTART_QUIET_MS;
}

/** Whether a host that has just stopped answering is the update working. */
export function restarting(now = Date.now()): boolean {
  return now < quietUntil;
}

/** The host answered again: whatever restart was expected has happened. */
export function forgetRestart(): void {
  quietUntil = 0;
}
