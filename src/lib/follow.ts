import { type RefCallback, useCallback, useRef } from "react";

/**
 * Keeps a transcript's newest line in view, and stops the moment the operator
 * looks away from it.
 *
 * Following the end is only ever wanted by somebody who is already at the end.
 * Anyone else is reading, and moving the page under a reader is the one thing
 * this must not do: an agent writing four hundred tokens is four hundred
 * chances to throw them back to the end of a cascade they were half way up.
 *
 * **Where the operator is, is not what a scroll event says.** A scroll event is
 * delivered after the fact, and a token committing in between arrives first. So
 * a wheel tick up, a token, and then the event, in that order: the transcript
 * is already back on the floor by the time anything is told the operator left
 * it, and the next tick starts the same race. That is the reported bug. Under
 * streaming text a trackpad could not climb out of the transcript at all, and
 * the eighty-pixel threshold this replaced only set the size of the pit.
 *
 * So the offset is compared rather than listened for. This hook remembers the
 * offset it wrote, and before writing again it checks the box is still there.
 * Above it, somebody else moved it, and that somebody is the operator: let go,
 * and leave the box alone. There is no window for a token to land in, because
 * the check and the write are the same statement.
 *
 * Scroll events keep one job, and it has no race in it: noticing the operator
 * come back. Nothing is following and nothing is being written by then, so a
 * late event costs nothing, and arriving at the end is worth being generous
 * about. Stopping a few pixels short of the end while scrolling down is
 * arriving at the end, and a transcript that then refuses to follow reads as
 * broken in the other direction.
 */

/** Coming down the page: this close to the end is arriving at it. */
const NEAR_END_PX = 24;

/**
 * Further above the offset the box was left at than this, and somebody moved
 * it. Not zero, because a scaled or zoomed layout leaves a fraction of a pixel
 * behind, and a fraction of a pixel is not a decision to read something.
 */
const AT_END_PX = 1;

export interface FollowBottom {
  /** Put this on the element that scrolls. */
  ref: RefCallback<HTMLElement>;
  /** That element, for anything that has to look inside it. */
  node: () => HTMLElement | null;
  /** Newest content is in; keep it in view if the operator is still there. */
  follow: () => void;
  /** Go to the newest and follow it, wherever the operator was. */
  pin: () => void;
}

export function useFollowBottom(): FollowBottom {
  const node = useRef<HTMLElement | null>(null);
  const following = useRef(true);
  /** The offset this hook last wrote, or null if it has not written one. */
  const written = useRef<number | null>(null);
  const lastTop = useRef(0);
  const frame = useRef(0);

  /**
   * Whether the box is still where this hook put it, and so whether following
   * it is still following the operator.
   *
   * Content disappearing from under them clamps the offset without anybody
   * scrolling, which is not a decision either: it leaves them at the end, which
   * is where they already were. That is the one upward move this takes over
   * rather than gives up on.
   */
  const ours = useCallback((box: HTMLElement) => {
    const put = written.current;
    if (put === null || box.scrollTop >= put - AT_END_PX) return true;
    if (box.scrollHeight - box.scrollTop - box.clientHeight > AT_END_PX) return false;
    written.current = box.scrollTop;
    return true;
  }, []);

  const release = useCallback(() => {
    following.current = false;
    written.current = null;
  }, []);

  const toEnd = useCallback(() => {
    const box = node.current;
    if (!box || !following.current) return;
    if (!ours(box)) return release();
    box.scrollTop = box.scrollHeight;
    // Read back rather than assumed: the browser clamps this to the end of the
    // box, and the end of the box is the number the next call compares against.
    written.current = box.scrollTop;
  }, [ours, release]);

  // A ref callback rather than an effect, because the element this listens to
  // is not the element that was here last render. A transcript is unmounted
  // whenever the pane shows something else (a pair thread, the activity board)
  // and comes back as a new node with the same class. An effect can only
  // re-bind on a dependency it was given, and the node it holds is not one: a
  // channel opened from the activity board bound its listener to the node that
  // had just been thrown away, so the operator was never noticed coming back.
  const ref = useCallback<RefCallback<HTMLElement>>(
    (box) => {
      node.current = box;
      if (!box) return;
      written.current = null;
      lastTop.current = box.scrollTop;
      const onScroll = () => {
        const top = box.scrollTop;
        const down = top >= lastTop.current;
        lastTop.current = top;
        // The box is measured only where the answer can change something.
        // Reading it is cheap while scrolling and expensive while text is
        // arriving, which is exactly when following is already settled.
        if (following.current) {
          if (!ours(box)) release();
          return;
        }
        if (down && box.scrollHeight - top - box.clientHeight <= NEAR_END_PX) {
          following.current = true;
          written.current = null;
        }
      };
      box.addEventListener("scroll", onScroll, { passive: true });
      return () => {
        node.current = null;
        box.removeEventListener("scroll", onScroll);
      };
    },
    [ours, release],
  );

  // Written now and again at the end of the frame. Now, because a frame drawn
  // short of the newest line is a visible stutter under streaming text; again,
  // because whatever landed after this call would otherwise sit below the fold
  // until the next one, which may be the token that never comes. Reading
  // `scrollHeight` forces layout, so a frame does this twice at most however
  // many times a stream asks.
  const follow = useCallback(() => {
    if (frame.current) return;
    toEnd();
    frame.current = requestAnimationFrame(() => {
      frame.current = 0;
      toEnd();
    });
  }, [toEnd]);

  const pin = useCallback(() => {
    following.current = true;
    written.current = null;
    toEnd();
  }, [toEnd]);

  const at = useCallback(() => node.current, []);

  return { ref, node: at, follow, pin };
}
