/**
 * What the operator is looking at, told to the host so an agent asked about
 * "this pane" can read what it is.
 *
 * Reported by the window with focus and nobody else: several windows can show
 * one host, and the one the operator touched last is the one they mean. A host
 * too old to know the command says so once, and this stops asking it rather
 * than failing on every click.
 */
import { useEffect, useRef } from "react";

import { api } from "./ipc";
import type { OperatorView } from "./types";

/** Long enough that tabbing through Settings reports the pane that stuck. */
const SETTLE_MS = 400;

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

  useEffect(() => {
    const report = () => {
      const now = latest.current;
      if (unsupported.current || !document.hasFocus() || sameView(sent.current, now)) return;
      sent.current = now;
      api.reportView(now).catch((refused: { kind?: string }) => {
        if (refused?.kind === "unknownCommand") unsupported.current = true;
        else sent.current = null;
      });
    };
    const timer = window.setTimeout(report, SETTLE_MS);
    // A window coming back to the front is the operator's again, even if what
    // it shows did not change while another window had them.
    const refocus = () => {
      sent.current = null;
      report();
    };
    window.addEventListener("focus", refocus);
    return () => {
      window.clearTimeout(timer);
      window.removeEventListener("focus", refocus);
    };
  }, [view.agentId, view.overlay, view.section, view.groupId]);
}
