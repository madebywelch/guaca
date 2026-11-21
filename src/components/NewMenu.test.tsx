import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NewMenu } from "./NewMenu";

function draw() {
  const onNewAgent = vi.fn();
  const onNewGroup = vi.fn();
  const onOpenCafeteria = vi.fn();
  render(
    <NewMenu onNewAgent={onNewAgent} onNewGroup={onNewGroup} onOpenCafeteria={onOpenCafeteria} />,
  );
  return {
    onNewAgent,
    onNewGroup,
    onOpenCafeteria,
    plus: screen.getByRole("button", { name: /make something/i }),
  };
}

describe("the plus", () => {
  it("offers nothing until it is opened", () => {
    draw();
    expect(screen.queryByRole("menu")).toBeNull();
  });

  it.each([
    ["New agent", "onNewAgent"],
    ["New group", "onNewGroup"],
    ["Open the cafeteria", "onOpenCafeteria"],
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

describe("where the plus menu grows from", () => {
  const window_ = { width: window.innerWidth, height: window.innerHeight };
  afterEach(() => {
    vi.restoreAllMocks();
    window.innerWidth = window_.width;
    window.innerHeight = window_.height;
  });

  function place(button: DOMRect, menu: { width: number; height: number }) {
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (
      this: HTMLElement,
    ) {
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
    window.innerWidth = 300;
    window.innerHeight = 200;
    place(box({ left: 250, top: 150, bottom: 182 }), { width: 200, height: 120 });
    const { plus } = draw();
    fireEvent.click(plus);
    expect(screen.getByRole("menu").style.getPropertyValue("--pop-origin")).toBe("bottom right");
  });
});
