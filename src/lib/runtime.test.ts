import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  subscribe: vi.fn(),
  bootstrap: vi.fn(),
  resynchronize: vi.fn(),
  applyEvent: vi.fn(),
  linkDown: vi.fn(),
  linkReconciling: vi.fn(),
  linkUp: vi.fn(),
  showFailure: vi.fn(),
}));

vi.mock("./ipc", () => ({ onRuntimeEvent: mocks.subscribe }));
vi.mock("./store", () => ({ useStore: { getState: () => mocks } }));

import { expectRestart, forgetRestart } from "./link";
import { useRuntime } from "./runtime";

function deferred() {
  let resolve!: () => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<void>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  forgetRestart();
  mocks.subscribe.mockResolvedValue(vi.fn());
  mocks.bootstrap.mockResolvedValue(undefined);
  mocks.resynchronize.mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("runtime reconciliation", () => {
  it("repeats a read for an event during bootstrap and only clears stale after it", async () => {
    const first = deferred();
    const second = deferred();
    mocks.bootstrap.mockReturnValue(first.promise);
    mocks.resynchronize.mockReturnValue(second.promise);
    const view = renderHook(() => useRuntime());
    await act(async () => {});
    const [event, open] = mocks.subscribe.mock.calls[0]!;
    act(() => {
      open();
      event({ type: "agentsChanged" });
    });
    await act(async () => first.resolve());
    expect(mocks.resynchronize).toHaveBeenCalledTimes(1);
    expect(mocks.linkUp).not.toHaveBeenCalled();
    await act(async () => second.resolve());
    expect(mocks.linkUp).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("retries a failed refresh without declaring the page current", async () => {
    mocks.resynchronize
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce(undefined);
    const view = renderHook(() => useRuntime());
    await act(async () => {});
    const [, open] = mocks.subscribe.mock.calls[0]!;
    await act(async () => open());
    expect(mocks.linkDown).toHaveBeenCalledOnce();
    expect(mocks.linkUp).not.toHaveBeenCalled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });
    expect(mocks.resynchronize).toHaveBeenCalledTimes(2);
    expect(mocks.linkUp).toHaveBeenCalledOnce();
    view.unmount();
  });

  it("marks the first open stale until its gap-filling read completes", async () => {
    const read = deferred();
    mocks.resynchronize.mockReturnValue(read.promise);
    const view = renderHook(() => useRuntime());
    await act(async () => {});
    const [, open] = mocks.subscribe.mock.calls[0]!;
    await act(async () => open());
    expect(mocks.linkReconciling).toHaveBeenCalledOnce();
    expect(mocks.linkUp).not.toHaveBeenCalled();
    await act(async () => read.resolve());
    expect(mocks.linkUp).toHaveBeenCalledOnce();
    view.unmount();
  });

  it("invalidates a read as soon as its socket closes", async () => {
    const read = deferred();
    mocks.resynchronize.mockReturnValueOnce(read.promise);
    const view = renderHook(() => useRuntime());
    await act(async () => {});
    const [, open, , close] = mocks.subscribe.mock.calls[0]!;
    await act(async () => open());
    const valid = mocks.resynchronize.mock.calls[0]![0];
    act(() => close());
    expect(valid()).toBe(false);
    await act(async () => read.resolve());
    expect(mocks.linkUp).not.toHaveBeenCalled();
    await act(async () => open());
    expect(mocks.linkUp).toHaveBeenCalledOnce();
    view.unmount();
  });

  it("invalidates an in-flight read and unsubscribes on teardown", async () => {
    const read = deferred();
    const stop = vi.fn();
    mocks.subscribe.mockResolvedValue(stop);
    mocks.bootstrap.mockReturnValue(read.promise);
    const view = renderHook(() => useRuntime());
    await act(async () => {});
    const valid = mocks.bootstrap.mock.calls[0]![0];
    view.unmount();
    expect(stop).toHaveBeenCalledOnce();
    expect(valid()).toBe(false);
    await act(async () => read.resolve());
    expect(mocks.resynchronize).not.toHaveBeenCalled();
  });

  it("reports a quiet restart when its deadline expires without a reconnect", async () => {
    expectRestart();
    const view = renderHook(() => useRuntime());
    await act(async () => {});
    const [, , lost, close] = mocks.subscribe.mock.calls[0]!;
    act(() => {
      close();
      lost({ kind: "unreachable" });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(179_999);
    });
    expect(mocks.linkDown).not.toHaveBeenCalled();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(mocks.linkDown).toHaveBeenCalledOnce();
    view.unmount();
  });

  it("follows an extended restart quiet period and cancels on teardown", async () => {
    expectRestart();
    const view = renderHook(() => useRuntime());
    await act(async () => {});
    const [, , lost, close] = mocks.subscribe.mock.calls[0]!;
    act(() => {
      close();
      lost({ kind: "unreachable" });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(90_000);
    });
    expectRestart();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(90_000);
    });
    expect(mocks.linkDown).not.toHaveBeenCalled();
    view.unmount();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(180_000);
    });
    expect(mocks.linkDown).not.toHaveBeenCalled();
  });
});
