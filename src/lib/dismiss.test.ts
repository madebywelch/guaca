import { fireEvent, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useDismiss } from "./dismiss";

describe("dismiss stack", () => {
  it("dismisses only the latest layer and restores its opener", () => {
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();
    expect(document.activeElement).toBe(opener);
    const lower = vi.fn();
    const upper = vi.fn();
    const first = renderHook(() => useDismiss(lower));
    const innerOpener = document.createElement("button");
    document.body.append(innerOpener);
    innerOpener.focus();
    const second = renderHook(() => useDismiss(upper));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(upper).toHaveBeenCalledTimes(1);
    expect(lower).not.toHaveBeenCalled();
    second.unmount();
    expect(document.activeElement).toBe(innerOpener);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(lower).toHaveBeenCalledTimes(1);
    first.unmount();
    expect(document.activeElement).toBe(opener);
    opener.remove();
    innerOpener.remove();
  });

  it("does not dismiss while composing or change order on rerender", () => {
    const lower = vi.fn();
    const upper = vi.fn();
    const first = renderHook(({ dismiss }) => useDismiss(dismiss), {
      initialProps: { dismiss: lower },
    });
    const second = renderHook(() => useDismiss(upper));
    first.rerender({ dismiss: lower });
    fireEvent.keyDown(window, { key: "Escape", isComposing: true });
    expect(upper).not.toHaveBeenCalled();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(upper).toHaveBeenCalledTimes(1);
    expect(lower).not.toHaveBeenCalled();
    second.unmount();
    first.unmount();
  });

  it("hands the opener to a replacement overlay when its own trigger disappears", () => {
    const opener = document.createElement("button");
    document.body.append(opener);
    opener.focus();
    const first = vi.fn();
    const second = vi.fn();
    const layer = renderHook(
      ({ replacement }) => {
        useDismiss(first, !replacement);
        useDismiss(second, replacement);
      },
      { initialProps: { replacement: false } },
    );
    const item = document.createElement("button");
    document.body.append(item);
    item.focus();
    item.remove();
    layer.rerender({ replacement: true });
    fireEvent.keyDown(window, { key: "Escape" });
    expect(second).toHaveBeenCalledTimes(1);
    layer.unmount();
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });
});
