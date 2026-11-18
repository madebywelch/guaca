import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { watchKeyboard } from "./keyboard";

const keyed = () => "keyboard" in document.documentElement.dataset;

describe("the keyboard mark", () => {
  let unwatch: () => void;

  beforeEach(() => {
    document.body.innerHTML = '<button type="button">Open Guaca</button>';
    unwatch = watchKeyboard(window);
  });

  afterEach(() => unwatch());

  it("is not set when a window opens and focuses a control on its own", () => {
    window.dispatchEvent(new FocusEvent("focus"));
    document.querySelector("button")?.focus();
    expect(keyed()).toBe(false);
  });

  it("is cleared by the pointer, by the window going away and by the window coming back", () => {
    for (const event of [
      new PointerEvent("pointerdown", { bubbles: true }),
      new FocusEvent("blur"),
      new FocusEvent("focus"),
    ]) {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
      expect(keyed()).toBe(true);
      (event.type === "pointerdown" ? document.body : window).dispatchEvent(event);
      expect(keyed()).toBe(false);
    }
  });

  it("is set by a key pressed anywhere in the window", () => {
    document
      .querySelector("button")
      ?.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab", bubbles: true }));
    expect(keyed()).toBe(true);
  });
});
