import { fireEvent, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { rememberDismissOpener, useDismiss } from "./dismiss";

describe("dismiss stack", () => {
  it("restores a clicked opener even when the browser did not focus it", () => {
    const previous = document.createElement("button");
    const opener = document.createElement("button");
    document.body.append(previous, opener);
    previous.focus();
    const layer = renderHook(({ enabled }) => useDismiss(vi.fn(), enabled), {
      initialProps: { enabled: false },
    });
    fireEvent.click(opener);
    layer.rerender({ enabled: true });
    layer.unmount();
    expect(document.activeElement).toBe(opener);
    previous.remove();
    opener.remove();
  });

  it("restores a recreated opener by its stable identifier after a microtask", async () => {
    const opener = document.createElement("button");
    opener.dataset.dismissId = 'row "one"';
    document.body.append(opener);
    rememberDismissOpener(opener);
    opener.remove();
    await Promise.resolve();
    const layer = renderHook(() => useDismiss(vi.fn()));
    const replacement = document.createElement("button");
    replacement.dataset.dismissId = 'row "one"';
    document.body.append(replacement);
    layer.unmount();
    expect(document.activeElement).toBe(replacement);
    replacement.remove();
  });

  it("does not hand an unused opener to a later overlay", async () => {
    const stale = document.createElement("button");
    const opener = document.createElement("button");
    document.body.append(stale, opener);
    rememberDismissOpener(stale);
    await new Promise((resolve) => setTimeout(resolve, 0));
    opener.focus();
    const layer = renderHook(() => useDismiss(vi.fn()));
    document.body.focus();
    layer.unmount();
    expect(document.activeElement).toBe(opener);
    stale.remove();
    opener.remove();
  });

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
