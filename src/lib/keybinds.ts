export interface Combo {
  key: string;
  mod?: boolean;
  ctrl?: boolean;
  shift?: boolean;
}

export const SURFACES = [
  "Anywhere",
  "The command menu",
  "Needs you",
  "A conversation",
  "Search",
  "Anything open",
] as const;

export type Surface = (typeof SURFACES)[number];

export interface Binding {
  id: string;
  what: string;
  combo: Combo;
  to?: { combo: Combo; joint: "pair" | "either" | "range" };
  where: Surface;
  fixed?: boolean;
}

export const IS_MAC = /mac/i.test(navigator.platform || navigator.userAgent);

export const FIND_KEY = IS_MAC ? "⌘F" : "Ctrl F";

export const MENU_KEY = IS_MAC ? "⌘K" : "Ctrl K";

export const BINDINGS: Binding[] = [
  {
    id: "menu",
    what: "Open the command menu to search, write to an agent, or run a command",
    combo: { key: "k", mod: true },
    where: "Anywhere",
  },
  {
    id: "agent",
    what: "Open the agent with that number",
    combo: { key: "1", mod: true },
    to: { combo: { key: "9", mod: true }, joint: "range" },
    where: "Anywhere",
  },
  {
    id: "step",
    what: "Open the next or previous agent",
    combo: { key: "Tab", ctrl: true, shift: false },
    to: { combo: { key: "Tab", ctrl: true, shift: true }, joint: "either" },
    where: "Anywhere",
  },
  {
    id: "search",
    what: "Search messages and files",
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
    id: "undo",
    what: "Take back the answer you just gave, while it offers Undo",
    combo: { key: "z", mod: true, shift: false },
    where: "Anywhere",
  },
  {
    id: "menu-move",
    what: "Move through the rows",
    combo: { key: "ArrowUp" },
    to: { combo: { key: "ArrowDown" }, joint: "pair" },
    where: "The command menu",
    fixed: true,
  },
  {
    id: "menu-do",
    what: "Do the selected row; with three words or more, send them",
    combo: { key: "Enter" },
    where: "The command menu",
    fixed: true,
  },
  {
    id: "recipient",
    what: "Change who you write to",
    combo: { key: "Tab" },
    to: { combo: { key: "Tab", shift: true }, joint: "either" },
    where: "The command menu",
    fixed: true,
  },
  {
    id: "menu-allow",
    what: "Allow the question the selected row belongs to",
    combo: { key: "Enter", mod: true },
    where: "The command menu",
    fixed: true,
  },
  {
    id: "menu-deny",
    what: "Deny it",
    combo: { key: "Backspace", mod: true },
    where: "The command menu",
    fixed: true,
  },
  {
    id: "queue",
    what: "From an empty composer, select the first question",
    combo: { key: "Escape" },
    where: "Needs you",
    fixed: true,
  },
  {
    id: "queue-move",
    what: "Select the next or the previous question",
    combo: { key: "j" },
    to: { combo: { key: "k" }, joint: "either" },
    where: "Needs you",
    fixed: true,
  },
  {
    id: "queue-allow",
    what: "Allow, or the selected question's first button",
    combo: { key: "Enter", mod: true },
    where: "Needs you",
    fixed: true,
  },
  {
    id: "queue-deny",
    what: "Deny, or clear the selected question",
    combo: { key: "Backspace", mod: true },
    where: "Needs you",
    fixed: true,
  },
  {
    id: "queue-option",
    what: "Choose one of the selected question's answers",
    combo: { key: "1" },
    to: { combo: { key: "9" }, joint: "range" },
    where: "Needs you",
    fixed: true,
  },
  {
    id: "queue-leave",
    what: "Go back to the composer",
    combo: { key: "Escape" },
    where: "Needs you",
    fixed: true,
  },
  {
    id: "answer",
    what: "From an empty box, answer the question that is waiting",
    combo: { key: "Tab" },
    where: "A conversation",
    fixed: true,
  },
  {
    id: "send",
    what: "Send the message",
    combo: { key: "Enter" },
    where: "A conversation",
    fixed: true,
  },
  {
    id: "newline",
    what: "Start a new line instead of sending",
    combo: { key: "Enter", shift: true },
    where: "A conversation",
    fixed: true,
  },
  {
    id: "mention",
    what: "Choose which agent to mention",
    combo: { key: "ArrowUp" },
    to: { combo: { key: "ArrowDown" }, joint: "pair" },
    where: "A conversation",
    fixed: true,
  },
  {
    id: "hit",
    what: "Move through the results",
    combo: { key: "ArrowUp" },
    to: { combo: { key: "ArrowDown" }, joint: "pair" },
    where: "Search",
    fixed: true,
  },
  {
    id: "scope",
    what: "Change what is being searched",
    combo: { key: "Tab" },
    to: { combo: { key: "Tab", shift: true }, joint: "either" },
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
  if (
    combo.ctrl
      ? !event.ctrlKey || event.metaKey
      : Boolean(combo.mod) !== (event.metaKey || event.ctrlKey)
  )
    return false;
  if (combo.shift !== undefined && combo.shift !== event.shiftKey) return false;
  return true;
}

export function heard(event: KeyboardEvent, binding: Binding): boolean {
  if (matches(event, binding.combo)) return true;
  const to = binding.to;
  if (!to) return false;
  if (to.joint !== "range") return matches(event, to.combo);
  const from = binding.combo.key;
  if (event.key.length !== 1 || event.key < from || event.key > to.combo.key) return false;
  return matches(event, { ...binding.combo, key: event.key });
}

export function bindingFor(event: KeyboardEvent): Binding | undefined {
  return BINDINGS.find((binding) => !binding.fixed && heard(event, binding));
}

export function agentNumber(event: KeyboardEvent): number | null {
  return bindingFor(event)?.id === "agent" ? Number(event.key) : null;
}

const NAMED: Record<string, string> = {
  Enter: "↩",
  Escape: "Esc",
  Tab: "Tab",
  ArrowUp: "↑",
  ArrowDown: "↓",
  Backspace: "⌫",
};

export function formatCombo(combo: Combo): string {
  const parts: string[] = [];
  if (combo.mod) parts.push(IS_MAC ? "⌘" : "Ctrl");
  if (combo.ctrl) parts.push(IS_MAC ? "⌃" : "Ctrl");
  if (combo.shift) parts.push(IS_MAC ? "⇧" : "Shift");
  parts.push(NAMED[combo.key] ?? combo.key.toUpperCase());
  return IS_MAC ? parts.join("") : parts.join("+");
}

export function comboLabel(binding: Binding): string {
  const first = formatCombo(binding.combo);
  const to = binding.to;
  if (!to) return first;
  const last = formatCombo(to.combo);
  if (to.joint === "pair") return first + last;
  if (to.joint === "either") return `${first} / ${last}`;
  return `${first} to ${last}`;
}

export function keyOf(id: string): string {
  const binding = BINDINGS.find((one) => one.id === id);
  if (!binding) throw new Error(`no binding named ${id}`);
  return formatCombo(binding.combo);
}
