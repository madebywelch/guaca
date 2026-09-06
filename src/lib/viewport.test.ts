import { afterEach, expect, it, vi } from "vitest";

import { followViewport } from "./viewport";

afterEach(() => vi.unstubAllGlobals());

it("leaves browsers without a visual viewport on the CSS fallback", () => {
  vi.stubGlobal("visualViewport", undefined);
  followViewport()();
  expect(document.documentElement.style.getPropertyValue("--viewport-height")).toBe("");
});

it("follows keyboard resize and pan, preserves pinch zoom, and removes listeners", () => {
  const viewport = Object.assign(new EventTarget(), { height: 844, offsetTop: 0, scale: 1 });
  vi.stubGlobal("visualViewport", viewport);
  const stop = followViewport();
  const style = document.documentElement.style;
  expect(style.getPropertyValue("--viewport-height")).toBe("844px");
  viewport.height = 380;
  viewport.dispatchEvent(new Event("resize"));
  expect(style.getPropertyValue("--viewport-height")).toBe("380px");
  viewport.offsetTop = 80;
  viewport.dispatchEvent(new Event("scroll"));
  expect(style.getPropertyValue("--viewport-top")).toBe("80px");
  viewport.scale = 2;
  viewport.height = 190;
  viewport.dispatchEvent(new Event("resize"));
  expect(style.getPropertyValue("--viewport-height")).toBe("380px");
  stop();
  viewport.scale = 1;
  viewport.dispatchEvent(new Event("resize"));
  expect(style.getPropertyValue("--viewport-height")).toBe("");
  expect(style.getPropertyValue("--viewport-top")).toBe("");
});
