import { dismissing } from "./dismiss";

export function focusWriting(): void {
  requestAnimationFrame(() => {
    if (dismissing()) return;
    document.querySelector<HTMLElement>("[data-field]:not([disabled])")?.focus();
  });
}

export function editingText(target: EventTarget | null): boolean {
  if (target instanceof HTMLTextAreaElement) return target.value !== "";
  if (target instanceof HTMLInputElement) return target.value !== "";
  return target instanceof HTMLElement && target.isContentEditable;
}

export function focusQuestion(id: string, within: ParentNode = document): boolean {
  const drawn = [...within.querySelectorAll<HTMLElement>("[data-question]")].find(
    (one) => one.dataset.question === id && !one.closest("[hidden]"),
  );
  const target = drawn?.hasAttribute("tabindex")
    ? drawn
    : drawn?.querySelector<HTMLElement>(":is(button, input):not(:disabled)");
  if (!target) return false;
  target.focus();
  return document.activeElement === target;
}
