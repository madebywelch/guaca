export interface Combo {
  key: string;
  mod?: boolean;
  shift?: boolean;
}

export const SURFACES = ["Anywhere", "A channel", "Search", "Anything open"] as const;

export type Surface = (typeof SURFACES)[number];

export interface Binding {
  id: string;
  what: string;
  combo: Combo;
  where: Surface;
  fixed?: boolean;
}

export const IS_MAC = /mac/i.test(navigator.platform || navigator.userAgent);

export const FIND_KEY = IS_MAC ? "⌘F" : "Ctrl F";

export const FIELD_KEY = IS_MAC ? "⌘K" : "Ctrl K";

export const BINDINGS: Binding[] = [
  {
    id: "field",
    what: "Write to an agent, or answer the one waiting",
    combo: { key: "k", mod: true },
    where: "Anywhere",
  },
  {
    id: "search",
    what: "Search agents, messages and actions",
    combo: { key: "f", mod: true },
    where: "Anywhere",
  },
  { id: "settings", what: "Open settings", combo: { key: ",", mod: true }, where: "Anywhere" },
  {
    id: "shortcuts",
    what: "Open this list",
    combo: { key: "/", mod: true },
    where: "Anywhere",
  },
  {
    id: "send",
    what: "Send the message",
    combo: { key: "Enter" },
    where: "A channel",
    fixed: true,
  },
  {
    id: "newline",
    what: "Start a new line instead of sending",
    combo: { key: "Enter", shift: true },
    where: "A channel",
    fixed: true,
  },
  {
    id: "mention",
    what: "Choose which agent to mention",
    combo: { key: "ArrowUp" },
    where: "A channel",
    fixed: true,
  },
  {
    id: "hit",
    what: "Move through the results",
    combo: { key: "ArrowDown" },
    where: "Search",
    fixed: true,
  },
  {
    id: "scope",
    what: "Change what is being searched",
    combo: { key: "Tab" },
    where: "Search",
    fixed: true,
  },
  {
    id: "open",
    what: "Open the result",
    combo: { key: "Enter" },
    where: "Search",
    fixed: true,
  },
  {
    id: "close",
    what: "Close it",
    combo: { key: "Escape" },
    where: "Anything open",
    fixed: true,
  },
];

export const GLOBAL = BINDINGS.filter((binding) => !binding.fixed).map((binding) => binding.id);

export function matches(event: KeyboardEvent, combo: Combo): boolean {
  const key = combo.key.length === 1 ? event.key.toLowerCase() : event.key;
  if (key !== combo.key) return false;
  if (event.altKey) return false;
  if (Boolean(combo.mod) !== (event.metaKey || event.ctrlKey)) return false;
  if (combo.shift !== undefined && combo.shift !== event.shiftKey) return false;
  return true;
}

export function bindingFor(event: KeyboardEvent): Binding | undefined {
  return BINDINGS.find((binding) => !binding.fixed && matches(event, binding.combo));
}

const NAMED: Record<string, string> = {
  Enter: "↩",
  Escape: "Esc",
  Tab: "Tab",
  ArrowUp: "↑",
  ArrowDown: "↓",
};

export function formatCombo(combo: Combo): string {
  const parts: string[] = [];
  if (combo.mod) parts.push(IS_MAC ? "⌘" : "Ctrl");
  if (combo.shift) parts.push(IS_MAC ? "⇧" : "Shift");
  parts.push(NAMED[combo.key] ?? combo.key.toUpperCase());
  return IS_MAC ? parts.join("") : parts.join("+");
}

export function comboLabel(binding: Binding): string {
  if (binding.id === "mention" || binding.id === "hit")
    return formatCombo({ key: "ArrowUp" }) + formatCombo({ key: "ArrowDown" });
  if (binding.id === "scope")
    return `${formatCombo({ key: "Tab" })} / ${formatCombo({ key: "Tab", shift: true })}`;
  return formatCombo(binding.combo);
}
