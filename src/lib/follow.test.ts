import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useFollowBottom } from "./follow";

/**
 * Whether a transcript is allowed to move under the operator.
 *
 * The report behind this: being scrolled up in a channel and thrown back to
 * the end of it, at times nobody could name. Two of them were nameable. A
 * channel opened from the activity board was listening for scrolls on a node
 * that had already been thrown away, so it never learned the operator had
 * moved at all; and the eighty-pixel threshold it used when it did work could
 * not be climbed out of while text was arriving, because every frame put the
 * view back on the floor before the next wheel tick was counted.
 *
 * jsdom does no layout, so the box is measured here instead. `scrollTop`
 * clamps and fires a scroll event exactly as a real one does, which is what
 * these rules are made of.
 */

/**
 * A scrolling box with a size, since jsdom gives every element none.
 *
 * `late` holds the scroll events back instead of dispatching them, which is
 * what a real browser does: the event is delivered after the fact, and a token
 * committing in between arrives first.
 */
function box(content: number, view: number, late = false) {
  const el = document.createElement("div");
  let height = content;
  let viewport = view;
  let top = 0;
  let writes = 0;
  let owed = 0;
  const announce = () => {
    if (late) owed += 1;
    else el.dispatchEvent(new Event("scroll"));
  };
  Object.defineProperty(el, "scrollHeight", { configurable: true, get: () => height });
  Object.defineProperty(el, "clientHeight", { configurable: true, get: () => viewport });
  Object.defineProperty(el, "scrollTop", {
    configurable: true,
    get: () => top,
    set: (next: number) => {
      writes += 1;
      const landed = Math.max(0, Math.min(next, height - viewport));
      if (landed === top) return;
      top = landed;
      announce();
    },
  });
  return {
    el,
    /** What the operator's wheel does: move, and the browser reports it. */
    scroll: (by: number) => {
      el.scrollTop = top + by;
    },
    /** A message or a token, arriving below the fold and announcing nothing. */
    grow: (by: number) => {
      height += by;
    },
    shrink: (by: number) => {
      height -= by;
      // The browser clamps an offset that no longer exists, and that is a
      // scroll event nobody asked for.
      if (top > height - viewport) {
        top = Math.max(0, height - viewport);
        announce();
      }
    },
    /**
     * The box itself getting smaller, which is what everything under a
     * transcript does to it: the composer growing a line, a trail of chips
     * landing, the working panel opening, the window being dragged shorter.
     * Nothing scrolls, so nothing is announced, and the end of the
     * conversation is simply somewhere else now.
     */
    narrow: (by: number) => {
      viewport -= by;
      resized(el);
    },
    widen: (by: number) => {
      viewport += by;
      if (top > height - viewport) {
        top = Math.max(0, height - viewport);
        announce();
      }
      resized(el);
    },
    /** What the browser gets round to telling us, whenever it does. */
    deliver: () => {
      for (; owed > 0; owed -= 1) el.dispatchEvent(new Event("scroll"));
    },
    top: () => top,
    /** How far the newest line sits below the bottom edge. */
    behind: () => height - top - viewport,
    /** How many times anything has moved the box. */
    writes: () => writes,
  };
}

/** Frames requested but not yet drawn, so a trailing write is a test's to make. */
let frames: FrameRequestCallback[] = [];

function flushFrames() {
  const due = frames;
  frames = [];
  for (const run of due) run(0);
}

/**
 * Who is watching which box for a change of size.
 *
 * The stub in `test-setup` reports nothing, which is right for a component that
 * only has to mount. This one is driven: `narrow` and `widen` are the box
 * changing size, and the browser telling whoever asked is the point.
 */
const watchers = new Map<Element, Set<ResizeObserverCallback>>();

function resized(el: Element) {
  for (const notify of watchers.get(el) ?? []) {
    notify([{ target: el } as ResizeObserverEntry], null as unknown as ResizeObserver);
  }
}

class WatchedResizeObserver {
  constructor(private readonly notify: ResizeObserverCallback) {}
  observe(target: Element) {
    const held = watchers.get(target) ?? new Set();
    held.add(this.notify);
    watchers.set(target, held);
  }
  unobserve(target: Element) {
    watchers.get(target)?.delete(this.notify);
  }
  disconnect() {
    for (const held of watchers.values()) held.delete(this.notify);
  }
}

function attach(content = 1000, view = 200, late = false) {
  const { result } = renderHook(() => useFollowBottom());
  const scroller = box(content, view, late);
  result.current.ref(scroller.el);
  return { ...scroller, hook: result.current };
}

beforeEach(() => {
  frames = [];
  watchers.clear();
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((run) => frames.push(run));
  vi.stubGlobal("ResizeObserver", WatchedResizeObserver);
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("following the newest line", () => {
  it("keeps up while the operator is at the end", () => {
    const view = attach();
    view.hook.pin();
    expect(view.behind()).toBe(0);

    view.grow(400);
    view.hook.follow();
    expect(view.behind()).toBe(0);
  });

  it("lets go the moment the operator scrolls up, however little", () => {
    // The whole report. Two pixels is a decision; the threshold this replaced
    // called it eighty and spent the difference dragging the reader back.
    const view = attach();
    view.hook.pin();

    view.scroll(-2);
    view.grow(400);
    view.hook.follow();
    flushFrames();

    expect(view.top()).toBe(798);
  });

  it("stays let go however much arrives", () => {
    const view = attach();
    view.hook.pin();
    view.scroll(-300);

    for (let i = 0; i < 20; i += 1) {
      view.grow(50);
      view.hook.follow();
      flushFrames();
    }
    expect(view.top()).toBe(500);
  });

  it("takes it back when the operator scrolls back to the end", () => {
    const view = attach();
    view.hook.pin();
    view.scroll(-400);
    view.scroll(400);

    view.grow(400);
    view.hook.follow();
    expect(view.behind()).toBe(0);
  });

  it("counts landing just short of the end as arriving at it", () => {
    // A flick that stops ten pixels out is the operator at the newest message.
    // Refusing to follow from there reads as broken in the other direction.
    const view = attach();
    view.hook.pin();
    view.scroll(-400);
    view.scroll(390);
    expect(view.behind()).toBe(10);

    view.grow(400);
    view.hook.follow();
    expect(view.behind()).toBe(0);
  });

  it("does not mistake scrolling down for arriving at the end", () => {
    const view = attach();
    view.hook.pin();
    view.scroll(-400);
    view.scroll(100);

    view.grow(400);
    view.hook.follow();
    flushFrames();
    expect(view.top()).toBe(500);
  });

  it("does not let go of the end for a nudge down toward it", () => {
    // A burst lands while the operator is at the bottom, and they scroll down
    // to read the rest of it. They were following the newest line and they
    // still are, whatever the arithmetic says about how far below them it is.
    const view = attach();
    view.hook.pin();
    view.grow(400);
    view.scroll(100);

    view.hook.follow();
    expect(view.behind()).toBe(0);
  });

  it("holds the end through content disappearing under it", () => {
    // Collapsing a tool trail shortens the transcript and the browser clamps
    // the offset it left behind. Nobody scrolled: the operator is still at the
    // newest line, and is still owed the next one.
    const view = attach();
    view.hook.pin();

    view.shrink(300);
    expect(view.behind()).toBe(0);

    view.grow(400);
    view.hook.follow();
    expect(view.behind()).toBe(0);
  });

  it("holds the end when the box shrinks under a follower", () => {
    // Everything below a transcript takes its height from the transcript: the
    // composer growing a line as a message is typed, a turn's chips landing,
    // the working panel opening, the window dragged shorter. Nothing scrolls
    // and no content arrives, so neither of the other two routes here fires,
    // and the newest line goes quietly under the fold and stays there until
    // the next token happens to land.
    const view = attach();
    view.hook.pin();
    expect(view.behind()).toBe(0);

    view.narrow(160);
    expect(view.behind()).toBe(0);
  });

  it("holds the end when the box grows back", () => {
    // Sending is the round trip: the composer grew while the message was
    // typed, and collapses to one line the moment it goes. The browser clamps
    // the offset on the way back, which leaves the operator at the end, and
    // this has to agree rather than call it somebody moving the box.
    const view = attach();
    view.hook.pin();
    view.narrow(160);
    expect(view.behind()).toBe(0);

    view.widen(160);
    expect(view.behind()).toBe(0);

    view.grow(400);
    view.hook.follow();
    expect(view.behind()).toBe(0);
  });

  it("leaves a reader alone when the box shrinks under them", () => {
    // The same event, and the reason it cannot simply write the end: somebody
    // who scrolled up and then typed a long message is still reading.
    const view = attach();
    view.hook.pin();
    view.scroll(-400);

    view.narrow(160);
    expect(view.top()).toBe(400);
  });

  it("does not move a reader who scrolled up after touching the end", () => {
    // Down to the newest line, then straight back up to keep reading. The
    // return is what puts this hook back in charge, and until it writes an
    // offset of its own there is nothing for the next scroll to be measured
    // against: every one of them read as its own, and the next message threw
    // the operator back to the floor.
    const view = attach();
    view.hook.pin();
    view.scroll(-400);
    view.scroll(400);
    view.scroll(-300);

    view.grow(400);
    view.hook.follow();
    flushFrames();

    expect(view.top()).toBe(500);
  });

  it("goes back to the end when the operator asks for it", () => {
    // Opening a channel, and sending a message. Both override where they were.
    const view = attach();
    view.hook.pin();
    view.scroll(-400);

    view.hook.pin();
    expect(view.behind()).toBe(0);
  });

  it("listens to the box that is on screen now, not the one that was", () => {
    // A transcript is unmounted whenever the pane shows a pair thread or the
    // activity board, and comes back as a different node. Bound to the old one,
    // nothing ever reported a scroll and every message won.
    const view = attach();
    view.hook.pin();

    const replacement = box(1000, 200);
    view.hook.ref(replacement.el);
    view.hook.pin();

    replacement.scroll(-300);
    replacement.grow(400);
    view.hook.follow();
    flushFrames();

    expect(replacement.top()).toBe(500);
  });

  it("hands back the box it is following, for anything that looks inside it", () => {
    const view = attach();
    expect(view.hook.node()).toBe(view.el);
  });
});

describe("what keeping up costs", () => {
  it("moves the box twice a frame however often text arrives", () => {
    // Reading `scrollHeight` forces the browser to lay the transcript out, and
    // a stream asks for this far more often than the screen redraws. One write
    // now, so the frame is not drawn short of the newest line, and one at the
    // end of it for whatever landed after.
    const view = attach();
    view.hook.pin();
    const before = view.writes();

    for (let i = 0; i < 30; i += 1) {
      view.grow(10);
      view.hook.follow();
    }
    expect(view.writes() - before).toBe(1);

    flushFrames();
    expect(view.writes() - before).toBe(2);
    // And the last of those tokens is on screen rather than below the fold.
    expect(view.behind()).toBe(0);
  });
});

describe("when the browser reports a scroll late", () => {
  it("lets go of a nudge the event has not arrived for yet", () => {
    // The report, in the order it actually happens: a wheel tick up, a token
    // committing, and the event about the tick after that. Anything waiting to
    // be told has already put the transcript back on the floor.
    const view = attach(1000, 200, true);
    view.hook.pin();
    view.deliver();

    view.scroll(-3);
    view.grow(20);
    view.hook.follow();
    flushFrames();

    expect(view.top()).toBe(797);
    // And the event, when it turns up, agrees with what was already decided.
    view.deliver();
    view.grow(20);
    view.hook.follow();
    expect(view.top()).toBe(797);
  });

  it("cannot be climbed out of one tick at a time either", () => {
    // Twelve ticks against text arriving on every one of them. The threshold
    // this replaced swallowed each tick whole and the operator went nowhere.
    const view = attach(4000, 400, true);
    view.hook.pin();
    view.deliver();
    const start = view.top();

    for (let i = 0; i < 12; i += 1) {
      view.scroll(-40);
      view.grow(20);
      view.hook.follow();
      flushFrames();
      view.deliver();
    }
    expect(view.top()).toBe(start - 480);
  });
});
