import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OperatorView } from "./types";

const reportView = vi.fn<(view: OperatorView) => Promise<void>>();
vi.mock("./ipc", () => ({ api: { reportView: (view: OperatorView) => reportView(view) } }));

import { useReportView } from "./view";

const channel: OperatorView = { agentId: "a1", overlay: null, section: null, groupId: null };
const limits: OperatorView = { ...channel, overlay: "settings", section: "limits" };

describe("telling the host what is on screen", () => {
  let focused = true;
  beforeEach(() => {
    vi.useFakeTimers();
    focused = true;
    vi.spyOn(document, "hasFocus").mockImplementation(() => focused);
    reportView.mockReset();
    reportView.mockResolvedValue();
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("reports the pane that stuck, once, and nothing while another window has focus", async () => {
    const { rerender } = renderHook((view: OperatorView) => useReportView(view), {
      initialProps: channel,
    });
    rerender({ ...limits, section: "provider" });
    rerender(limits);
    await act(async () => vi.advanceTimersByTime(500));
    expect(reportView.mock.calls).toEqual([[limits]]);

    rerender({ ...limits });
    await act(async () => vi.advanceTimersByTime(500));
    expect(reportView).toHaveBeenCalledTimes(1);

    focused = false;
    rerender(channel);
    await act(async () => vi.advanceTimersByTime(500));
    expect(reportView).toHaveBeenCalledTimes(1);

    // Coming back to the front is a report, even of what it already said.
    focused = true;
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(reportView).toHaveBeenLastCalledWith(channel);
  });

  it("stops asking a host that does not know the command", async () => {
    reportView.mockRejectedValue({ kind: "unknownCommand", message: "no such command" });
    const { rerender } = renderHook((view: OperatorView) => useReportView(view), {
      initialProps: channel,
    });
    await act(async () => vi.advanceTimersByTime(500));
    rerender(limits);
    await act(async () => vi.advanceTimersByTime(500));
    expect(reportView).toHaveBeenCalledTimes(1);
  });
});
