import { renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useFollowBottom } from "./follow";

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
    scroll: (by: number) => {
      el.scrollTop = top + by;
    },
    grow: (by: number) => {
      height += by;
    },
    shrink: (by: number) => {
      height -= by;
      if (top > height - viewport) {
        top = Math.max(0, height - viewport);
        announce();
      }
    },
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
    deliver: () => {
      for (; owed > 0; owed -= 1) el.dispatchEvent(new Event("scroll"));
    },
    top: () => top,
    behind: () => height - top - viewport,
    writes: () => writes,
  };
}

let frames: FrameRequestCallback[] = [];

function flushFrames() {
  const due = frames;
  frames = [];
  for (const run of due) run(0);
}

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
    const view = attach();
    view.hook.pin();
    view.grow(400);
    view.scroll(100);

    view.hook.follow();
    expect(view.behind()).toBe(0);
  });

  it("holds the end through content disappearing under it", () => {
    const view = attach();
    view.hook.pin();

    view.shrink(300);
    expect(view.behind()).toBe(0);

    view.grow(400);
    view.hook.follow();
    expect(view.behind()).toBe(0);
  });

  it("holds the end when the box shrinks under a follower", () => {
    const view = attach();
    view.hook.pin();
    expect(view.behind()).toBe(0);

    view.narrow(160);
    expect(view.behind()).toBe(0);
  });

  it("holds the end when the box grows back", () => {
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
    const view = attach();
    view.hook.pin();
    view.scroll(-400);

    view.narrow(160);
    expect(view.top()).toBe(400);
  });

  it("does not move a reader who scrolled up after touching the end", () => {
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
    const view = attach();
    view.hook.pin();
    view.scroll(-400);

    view.hook.pin();
    expect(view.behind()).toBe(0);
  });

  it("listens to the box that is on screen now, not the one that was", () => {
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
    expect(view.behind()).toBe(0);
  });
});

describe("when the browser reports a scroll late", () => {
  it("lets go of a nudge the event has not arrived for yet", () => {
    const view = attach(1000, 200, true);
    view.hook.pin();
    view.deliver();

    view.scroll(-3);
    view.grow(20);
    view.hook.follow();
    flushFrames();

    expect(view.top()).toBe(797);
    view.deliver();
    view.grow(20);
    view.hook.follow();
    expect(view.top()).toBe(797);
  });

  it("cannot be climbed out of one tick at a time either", () => {
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
