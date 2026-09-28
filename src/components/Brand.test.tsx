import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { HOLD, SETTLED } from "../lib/brand";
import { Brand } from "./Brand";

vi.mock("../avatars/clock", async (original) => ({
  ...(await original<typeof import("../avatars/clock")>()),
  join: () => () => {},
}));

const still = { reduce: false };
vi.mock("../lib/motion", () => ({ prefersReducedMotion: () => still.reduce }));

beforeEach(() => {
  still.reduce = false;
  vi.useFakeTimers({
    toFake: [
      "setTimeout",
      "clearTimeout",
      "performance",
      "requestAnimationFrame",
      "cancelAnimationFrame",
    ],
  });
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

const wait = (seconds: number) => act(() => vi.advanceTimersByTime(seconds * 1000));
const corner = (container: HTMLElement) => container.querySelector(".brand") as HTMLElement;
const moods = (container: HTMLElement) =>
  Array.from(container.querySelectorAll(".avatar")).map((a) => a.getAttribute("data-mood"));

describe("the corner", () => {
  it("names the rail for a screen reader whether or not the name is showing", () => {
    const { container } = render(<Brand waiting={false} />);
    expect(screen.getByText("Guaca")).toBeTruthy();
    expect(corner(container).hasAttribute("data-named")).toBe(false);
    expect(container.querySelector(".brand__pair")?.getAttribute("aria-hidden")).toBe("true");
  });

  it("writes the name in after the catch, and folds it away at the hold", () => {
    const { container } = render(<Brand waiting={false} />);
    wait(1.9);
    expect(corner(container).hasAttribute("data-named")).toBe(false);
    wait(0.2);
    expect(corner(container).hasAttribute("data-named")).toBe(true);
    wait(HOLD - 2.1);
    expect(corner(container).hasAttribute("data-named")).toBe(false);
    expect(screen.getByText("Guaca")).toBeTruthy();
  });

  it("brings the pair in one at a time", () => {
    const { container } = render(<Brand waiting={false} />);
    const seats = () =>
      Array.from(container.querySelectorAll(".brand__seat")).map((s) =>
        s.hasAttribute("data-shown"),
      );
    expect(seats()).toEqual([false, false]);
    wait(0.1);
    expect(seats()).toEqual([true, false]);
    wait(0.4);
    expect(seats()).toEqual([true, true]);
  });

  it("brings the name back for the pointer, and puts it away when the pointer goes", () => {
    const { container } = render(<Brand waiting={false} />);
    wait(HOLD + 1);
    fireEvent.pointerEnter(corner(container));
    expect(corner(container).hasAttribute("data-named")).toBe(true);
    fireEvent.pointerLeave(corner(container));
    expect(corner(container).hasAttribute("data-named")).toBe(false);
  });

  it("shows a turn parked on the operator on the first of them, once the launch is over", () => {
    const { container } = render(<Brand waiting={true} />);
    wait(1);
    expect(moods(container)).not.toContain("blocked");
    wait(SETTLED);
    expect(moods(container)).toEqual(["blocked", "idle"]);
  });

  it("lets the face go when the turn is answered", () => {
    const { container, rerender } = render(<Brand waiting={true} />);
    wait(SETTLED + 1);
    rerender(<Brand waiting={false} />);
    expect(moods(container)).toEqual(["idle", "idle"]);
  });
});

describe("reduced motion", () => {
  it("never performs: both there at once, the name only for the pointer", () => {
    still.reduce = true;
    const { container } = render(<Brand waiting={false} />);
    expect(
      Array.from(container.querySelectorAll(".brand__seat")).every((s) =>
        s.hasAttribute("data-shown"),
      ),
    ).toBe(true);
    wait(3);
    expect(corner(container).hasAttribute("data-named")).toBe(false);
    fireEvent.pointerEnter(corner(container));
    expect(corner(container).hasAttribute("data-named")).toBe(true);
  });
});
