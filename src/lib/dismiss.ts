import { useEffect, useLayoutEffect, useRef } from "react";

const stack: { dismiss: () => void }[] = [];
let handoff: HTMLElement | null = null;

function onKey(event: KeyboardEvent) {
  if (event.key !== "Escape" || event.isComposing || event.defaultPrevented) return;
  const top = stack.at(-1);
  if (!top) return;
  event.preventDefault();
  event.stopImmediatePropagation();
  top.dismiss();
}

export function useDismiss(dismiss: () => void, enabled = true) {
  const latest = useRef(dismiss);
  const current = useRef<{ dismiss: () => void } | null>(null);
  const restore = useRef<HTMLElement | null>(null);
  latest.current = dismiss;
  useLayoutEffect(() => {
    if (!enabled) return;
    const active = document.activeElement;
    const previous = active instanceof HTMLElement && active !== document.body ? active : handoff;
    const entry = { dismiss: () => latest.current() };
    current.current = entry;
    if (stack.length === 0) window.addEventListener("keydown", onKey);
    stack.push(entry);
    return () => {
      const index = stack.indexOf(entry);
      const wasTop = index === stack.length - 1;
      if (index !== -1) stack.splice(index, 1);
      if (current.current === entry) current.current = null;
      if (stack.length === 0) window.removeEventListener("keydown", onKey);
      if (wasTop && previous instanceof HTMLElement) {
        restore.current = previous;
        handoff = previous;
      }
    };
  }, [enabled]);
  useEffect(() => {
    if (!enabled) return;
    return () => {
      if (restore.current?.isConnected) restore.current.focus();
      if (handoff === restore.current) handoff = null;
      restore.current = null;
    };
  }, [enabled]);
  return () => current.current !== null && stack.at(-1) === current.current;
}
