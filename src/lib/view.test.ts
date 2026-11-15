import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { OperatorView } from "./types";

type Presence = { focused: boolean; client: string };
const reportView = vi.fn<(view: OperatorView, presence?: Presence) => Promise<void>>();
vi.mock("./ipc", () => ({
  api: {
    reportView: (view: OperatorView, presence?: Presence) => reportView(view, presence),
  },
}));

import { PAGE_ID } from "./transport";
import { HEARTBEAT_MS, useReportView } from "./view";

const channel: OperatorView = { agentId: "a1", overlay: null, section: null, groupId: null };
const limits: OperatorView = { ...channel, overlay: "settings", section: "limits" };
const here: Presence = { focused: true, client: PAGE_ID };
const gone: Presence = { focused: false, client: PAGE_ID };

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
    expect(reportView.mock.calls).toEqual([[limits, here]]);

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
    expect(reportView).toHaveBeenLastCalledWith(channel, here);
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

describe("whether anyone is at the window", () => {
  // The Host counts the operator away two minutes after the last report with
  // focus. These are the reports that keep that true and no truer.
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

  it("says so again at least once a minute while the window keeps focus", async () => {
    expect(HEARTBEAT_MS).toBeLessThanOrEqual(60_000);
    renderHook(() => useReportView(channel));
    await act(async () => vi.advanceTimersByTime(500));
    expect(reportView).toHaveBeenCalledTimes(1);

    // Reading one channel for ten minutes is being here for ten minutes.
    await act(async () => vi.advanceTimersByTime(10 * 60_000));
    expect(reportView.mock.calls.length).toBeGreaterThanOrEqual(11);
    for (const call of reportView.mock.calls) expect(call).toEqual([channel, here]);
  });

  it("says nothing on the beat while another window has focus", async () => {
    focused = false;
    renderHook(() => useReportView(channel));
    await act(async () => vi.advanceTimersByTime(5 * 60_000));
    expect(reportView).not.toHaveBeenCalled();
  });

  it("reports losing focus, and the next focus is a report again", async () => {
    renderHook(() => useReportView(channel));
    await act(async () => vi.advanceTimersByTime(500));

    focused = false;
    await act(async () => {
      window.dispatchEvent(new Event("blur"));
      vi.advanceTimersByTime(1);
    });
    expect(reportView).toHaveBeenLastCalledWith(channel, gone);

    focused = true;
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(reportView).toHaveBeenLastCalledWith(channel, here);
  });

  it("does not count focus moving into a page inside the window as leaving it", async () => {
    // An agent's page is a frame in this document: the window blurs, and the
    // document still has focus.
    renderHook(() => useReportView(channel));
    await act(async () => vi.advanceTimersByTime(500));
    await act(async () => {
      window.dispatchEvent(new Event("blur"));
      vi.advanceTimersByTime(1);
    });
    expect(reportView.mock.calls).toEqual([[channel, here]]);
  });

  it("names the page the same way on every report, so the Host can tell windows apart", async () => {
    expect(PAGE_ID).toMatch(/^[A-Za-z0-9_-]{1,64}$/);
    renderHook(() => useReportView(channel));
    await act(async () => vi.advanceTimersByTime(HEARTBEAT_MS * 2 + 500));
    focused = false;
    await act(async () => {
      window.dispatchEvent(new Event("blur"));
      vi.advanceTimersByTime(1);
    });
    expect(new Set(reportView.mock.calls.map(([, presence]) => presence?.client))).toEqual(
      new Set([PAGE_ID]),
    );
  });

  describe("a page that is hidden and still says it has focus", () => {
    // What a Mac's webview does when Guaca is hidden: no blur, the document
    // keeps reporting focus, and only the visibility changes.
    let visibility: DocumentVisibilityState = "visible";
    beforeEach(() => {
      visibility = "visible";
      vi.spyOn(document, "visibilityState", "get").mockImplementation(() => visibility);
    });
    const hide = async () => {
      visibility = "hidden";
      await act(async () => document.dispatchEvent(new Event("visibilitychange")));
    };

    it("reports losing focus when it is hidden", async () => {
      renderHook(() => useReportView(channel));
      await act(async () => vi.advanceTimersByTime(500));
      await hide();
      expect(reportView).toHaveBeenLastCalledWith(channel, gone);
    });

    it("says nothing on the beat while it is hidden, so the Host counts the operator away", async () => {
      renderHook(() => useReportView(channel));
      await act(async () => vi.advanceTimersByTime(500));
      await hide();
      const said = reportView.mock.calls.length;
      await act(async () => vi.advanceTimersByTime(5 * 60_000));
      expect(reportView).toHaveBeenCalledTimes(said);
    });

    it("reports nothing for what it shows while it is hidden", async () => {
      const { rerender } = renderHook((view: OperatorView) => useReportView(view), {
        initialProps: channel,
      });
      await act(async () => vi.advanceTimersByTime(500));
      await hide();
      const said = reportView.mock.calls.length;
      rerender(limits);
      await act(async () => {
        window.dispatchEvent(new Event("focus"));
        vi.advanceTimersByTime(500);
      });
      expect(reportView).toHaveBeenCalledTimes(said);
    });

    it("is the operator's again once it is shown with focus", async () => {
      renderHook(() => useReportView(channel));
      await act(async () => vi.advanceTimersByTime(500));
      await hide();
      visibility = "visible";
      await act(async () => document.dispatchEvent(new Event("visibilitychange")));
      expect(reportView).toHaveBeenLastCalledWith(channel, here);
    });
  });

  it("stops the beat when the page goes away", async () => {
    const { unmount } = renderHook(() => useReportView(channel));
    await act(async () => vi.advanceTimersByTime(500));
    unmount();
    await act(async () => vi.advanceTimersByTime(5 * 60_000));
    expect(reportView).toHaveBeenCalledTimes(1);
  });
});
