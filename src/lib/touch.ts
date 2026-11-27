import { useSyncExternalStore } from "react";

const TOUCH = "(hover: none) and (pointer: coarse)";

export function onTouchScreen(): boolean {
  return window.matchMedia?.(TOUCH).matches ?? false;
}

function watch(changed: () => void): () => void {
  const query = window.matchMedia?.(TOUCH);
  query?.addEventListener?.("change", changed);
  return () => query?.removeEventListener?.("change", changed);
}

export function useTouchScreen(): boolean {
  return useSyncExternalStore(watch, onTouchScreen, () => false);
}
