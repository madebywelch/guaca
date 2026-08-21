/**
 * The one question everything animated in this app has to ask.
 *
 * Asked at the moment of the movement rather than held in state: the operator
 * can change the setting while the app is running, and reading it late means
 * every animation from the next one onwards is right without anything having to
 * subscribe to it.
 */
export function prefersReducedMotion(): boolean {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}
