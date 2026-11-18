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
