/**
 * What the operator is looking at, and whether they are looking at all, told
 * to the host.
 *
 * Two readers. An agent asked about "this pane" reads what the focused window
 * shows. The Host's outbox reads presence: whether anyone is at a window, and
 * which channel is on screen, to decide what is worth a notification
 * (`domain/view.rs`, `domain/outbox.rs`).
 *
 * The presence contract is the Host's, and this hook is the only thing that
 * keeps it. The page names itself once per load, so several windows on one
 * Host are told apart. A window reports with focus when it gets focus, when
 * what it shows settles, and again at least once a minute while it keeps
 * focus; it reports without focus when it loses it. The Host counts the
 * operator away two minutes after the last report with focus, which is what
 * lets a window that crashed or slept count as gone, so the heartbeat is what
 * keeps someone reading one channel for ten minutes from counting as away.
 *
 * Focus is the document's focus while the page is visible, and only then. On
 * a Mac, hiding Guaca (Command-H, or the app hidden any other way) fires no
 * blur in the webview and leaves `document.hasFocus()` true for as long as it
 * stays hidden; only `visibilitychange` says anything. Asked alone, the
 * heartbeat kept a hidden Guaca present forever, and finished work never
 * interrupted anyone who had looked at it once.
 *
 * Only the window reports. The menu bar panel never calls this, so it neither
 * counts as the operator being present nor puts a channel on screen.
 *
 * A host too old to know the command says so once, and this stops asking it
 * rather than failing on every click.
 */
import { useEffect, useRef } from "react";

import { api } from "./ipc";
import { PAGE_ID } from "./transport";
import type { OperatorView } from "./types";

/** Long enough that tabbing through Settings reports the pane that stuck. */
const SETTLE_MS = 400;

/**
 * How often a window with focus says so again. Well inside the Host's two
 * minutes, so one heartbeat delayed by a busy main thread does not make the
 * operator away while they are reading.
 */
export const HEARTBEAT_MS = 45_000;

/** Whether the operator is in this window: it has focus and can be seen. */
function present(): boolean {
  return document.hasFocus() && document.visibilityState !== "hidden";
}

export function sameView(a: OperatorView | null, b: OperatorView): boolean {
  return (
    a !== null &&
    a.agentId === b.agentId &&
    a.overlay === b.overlay &&
    a.section === b.section &&
    a.groupId === b.groupId
  );
}

export function useReportView(view: OperatorView): void {
  const latest = useRef(view);
  latest.current = view;
  const sent = useRef<OperatorView | null>(null);
  const unsupported = useRef(false);

  // Built once and kept: it reads only refs, so the listeners below can be
  // added once for the life of the page.
  const report = useRef((focused: boolean, again: boolean) => {
    const now = latest.current;
    if (unsupported.current) return;
    if (focused && !again && sameView(sent.current, now)) return;
    sent.current = focused ? now : null;
    api.reportView(now, { focused, client: PAGE_ID }).catch((refused: { kind?: string }) => {
      if (refused?.kind === "unknownCommand") unsupported.current = true;
      else sent.current = null;
    });
  }).current;

  useEffect(() => {
    const timer = window.setTimeout(() => {
      if (present()) report(true, false);
    }, SETTLE_MS);
    return () => window.clearTimeout(timer);
  }, [view.agentId, view.overlay, view.section, view.groupId, report]);

  useEffect(() => {
    // A window coming back to the front is the operator's again, even if what
    // it shows did not change while another window had them.
    const focus = () => {
      if (present()) report(true, true);
    };
    // Focus moving into a page an agent wrote, which is a frame inside this
    // document, blurs the window and leaves the document focused. The operator
    // is still here, so a blur counts only once the document agrees.
    let checking: number | undefined;
    const blur = () => {
      window.clearTimeout(checking);
      checking = window.setTimeout(() => {
        if (!present()) report(false, true);
      }, 0);
    };
    // Hidden is leaving, and shown again with focus is coming back.
    const shown = () => {
      if (document.visibilityState === "hidden") report(false, true);
      else focus();
    };
    const beat = window.setInterval(() => {
      if (present()) report(true, true);
    }, HEARTBEAT_MS);
    window.addEventListener("focus", focus);
    window.addEventListener("blur", blur);
    document.addEventListener("visibilitychange", shown);
    return () => {
      window.clearTimeout(checking);
      window.clearInterval(beat);
      window.removeEventListener("focus", focus);
      window.removeEventListener("blur", blur);
      document.removeEventListener("visibilitychange", shown);
    };
  }, [report]);
}
