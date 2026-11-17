import { useEffect, useRef } from "react";

import { api } from "./ipc";
import { PAGE_ID } from "./transport";
import type { OperatorView } from "./types";

const SETTLE_MS = 400;

export const HEARTBEAT_MS = 45_000;

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
    const focus = () => {
      if (present()) report(true, true);
    };
    let checking: number | undefined;
    const blur = () => {
      window.clearTimeout(checking);
      checking = window.setTimeout(() => {
        if (!present()) report(false, true);
      }, 0);
    };
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
