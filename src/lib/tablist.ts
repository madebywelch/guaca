import type { KeyboardEvent } from "react";

export function moveAlongTabs(event: KeyboardEvent<HTMLElement>): void {
  const all = [...event.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]:not(:disabled)')];
  const at = all.findIndex((tab) => tab.getAttribute("aria-selected") === "true");
  const to =
    event.key === "ArrowDown" || event.key === "ArrowRight"
      ? (at + 1) % all.length
      : event.key === "ArrowUp" || event.key === "ArrowLeft"
        ? (at - 1 + all.length) % all.length
        : event.key === "Home"
          ? 0
          : event.key === "End"
            ? all.length - 1
            : null;
  if (to === null) return;
  event.preventDefault();
  all[to]?.focus();
  all[to]?.click();
}
