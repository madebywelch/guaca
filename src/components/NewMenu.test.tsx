import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NewMenu } from "./NewMenu";

/**
 * The plus is the only way to make an agent or a crew from the main window, so
 * the two things worth holding are that both are behind it and that it closes.
 * A menu that will not shut sits over the transcript it opened on top of.
 */

function draw() {
  const onNewAgent = vi.fn();
  const onNewGroup = vi.fn();
  render(<NewMenu onNewAgent={onNewAgent} onNewGroup={onNewGroup} />);
  return { onNewAgent, onNewGroup, plus: screen.getByRole("button", { name: /make something/i }) };
}

describe("the plus", () => {
  it("offers nothing until it is opened", () => {
    draw();
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it.each([
    ["New agent", "onNewAgent"],
    ["New group", "onNewGroup"],
  ] as const)("runs %s and closes", (label, prop) => {
    const handles = draw();
    fireEvent.click(handles.plus);
    fireEvent.click(screen.getByRole("menuitem", { name: new RegExp(label, "i") }));

    expect(handles[prop]).toHaveBeenCalledOnce();
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("says whether it is open, for anything that cannot see it", () => {
    const { plus } = draw();
    expect(plus.getAttribute("aria-expanded")).toBe("false");
    fireEvent.click(plus);
    expect(plus.getAttribute("aria-expanded")).toBe("true");
  });

  it("closes on Escape, and on a click that lands away from it", () => {
    const { plus } = draw();

    fireEvent.click(plus);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.queryByRole("menu")).toBeNull();

    fireEvent.click(plus);
    fireEvent.click(screen.getByRole("button", { name: "Close menu" }));
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it("stops listening once it is shut", () => {
    // The listeners are registered on the window and removed by reference. An
    // inline arrow in both calls leaves one behind on every open, and the leak
    // is invisible until something else is drawing where the menu was.
    const added = vi.spyOn(window, "addEventListener");
    const removed = vi.spyOn(window, "removeEventListener");
    const { plus } = draw();

    fireEvent.click(plus);
    const bound = added.mock.calls.filter(([kind]) => kind === "resize");
    fireEvent.keyDown(window, { key: "Escape" });

    expect(bound).toHaveLength(1);
    const listener = bound[0]?.[1];
    expect(removed.mock.calls.some(([kind, fn]) => kind === "resize" && fn === listener)).toBe(
      true,
    );
    added.mockRestore();
    removed.mockRestore();
  });
});

/**
 * Where the menu grows from.
 *
 * Every surface in the app arrives on one keyframe, and the only thing that
 * tells a menu apart from a dialog is its origin: it grows out of the corner
 * nearest the button that was pressed. Taken from the button rather than from
 * where the menu actually landed, a menu pulled back off the window edge
 * animates in from a corner it is no longer anywhere near, which reads as the
 * panel sliding across the screen.
 */
describe("where the plus menu grows from", () => {
  // Both stubs are global. Left in place they would decide the layout of every
  // test written after this one, in a file where nothing else measures.
  const window_ = { width: window.innerWidth, height: window.innerHeight };
  afterEach(() => {
    vi.restoreAllMocks();
    window.innerWidth = window_.width;
    window.innerHeight = window_.height;
  });

  /** jsdom lays nothing out, so both boxes have to be stated. */
  function place(button: DOMRect, menu: { width: number; height: number }) {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
      // By role: the plus and the menu it opens carry the same label.
      const box =
        this.getAttribute("role") === "menu"
          ? { ...menu, top: 0, left: 0, bottom: menu.height, right: menu.width }
          : button;
      return box as DOMRect;
    });
  }

  const box = (over: Partial<DOMRect>) =>
    ({ top: 0, left: 0, bottom: 0, right: 0, ...over }) as DOMRect;

  it("is the corner under the plus when the menu fits there", () => {
    place(box({ left: 40, top: 20, bottom: 52 }), { width: 200, height: 120 });
    const { plus } = draw();
    fireEvent.click(plus);
    expect(screen.getByRole("menu").style.getPropertyValue("--pop-origin")).toBe("top left");
  });

  it("swaps to the far corner when the window pulls it back", () => {
    // A plus near the bottom right of a small window: the menu cannot open
    // down and to the right, so it lands above and to the left of where it
    // asked for, and that is the corner it has to grow from.
    window.innerWidth = 300;
    window.innerHeight = 200;
    place(box({ left: 250, top: 150, bottom: 182 }), { width: 200, height: 120 });
    const { plus } = draw();
    fireEvent.click(plus);
    expect(screen.getByRole("menu").style.getPropertyValue("--pop-origin")).toBe("bottom right");
  });
});
