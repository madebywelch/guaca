import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

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
