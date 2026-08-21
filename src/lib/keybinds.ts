/**
 * Every key the app answers to, in one list.
 *
 * The list exists because the app already answered to a dozen keys and said so
 * nowhere. Discoverability is the whole feature: an operator who cannot find a
 * shortcut does not have it.
 *
 * It is a reference, not a rebinding system, and the difference is deliberate.
 * Nine of these are handled by the component that owns the surface they work
 * on: Escape by whatever is open, Enter and the arrows by the composer and the
 * palette. Rebinding would mean routing all nine through one dispatcher, and a
 * setting that appeared to rebind a key and only moved one of them is worse
 * than no setting at all. So the fixed ones are listed as fixed, and only the
 * three that are genuinely global are matched from this table.
 *
 * What that buys, beyond the panel: `mod` here means Command or Control on
 * every platform, both accepted, which is the rule the one existing shortcut
 * already followed and the reason it is written down once now.
 */

export interface Combo {
  /** `event.key`, lowercased for letters and verbatim for named keys. */
  key: string;
  /** Command or Control. Either one, on every platform. */
  mod?: boolean;
  shift?: boolean;
}

/** The surfaces a binding belongs to, in the order the panel lists them. */
export const SURFACES = ["Anywhere", "A channel", "Search", "Anything open"] as const;

export type Surface = (typeof SURFACES)[number];

export interface Binding {
  id: string;
  /** What it does, in the operator's words. */
  what: string;
  combo: Combo;
  where: Surface;
  /**
   * True when the key is owned by the surface it works on rather than by the
   * one global handler, and therefore cannot be changed from here. Listing it
   * anyway is the point: the panel is a map of what the app responds to, not of
   * what happens to be configurable.
   */
  fixed?: boolean;
}

const IS_MAC = /mac/i.test(navigator.platform || navigator.userAgent);

export const BINDINGS: Binding[] = [
  {
    id: "search",
    what: "Search agents, messages and actions",
    combo: { key: "k", mod: true },
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

/** The ids the one global handler dispatches. Everything else is `fixed`. */
export const GLOBAL = BINDINGS.filter((binding) => !binding.fixed).map((binding) => binding.id);

/**
 * True when this keystroke is that binding.
 *
 * Either modifier satisfies `mod`, on every platform: an operator who learned
 * the shortcut on a laptop should not have to learn it again on a desktop.
 * `shift` is checked only when the binding names it, because a shortcut that
 * silently stopped working with caps lock or a shifted layout would be
 * indistinguishable from a broken build.
 *
 * Alt never matches anything, and that is not fussiness. On Windows and Linux
 * AltGr arrives as Control *and* Alt, so on a layout where AltGr and a key
 * produce a character, typing that character would look exactly like a
 * shortcut: the global handler calls `preventDefault`, the character never
 * arrives, and a dialog opens instead. No binding here wants Alt, so the
 * cheapest correct rule is that none of them tolerate it.
 */
export function matches(event: KeyboardEvent, combo: Combo): boolean {
  const key = combo.key.length === 1 ? event.key.toLowerCase() : event.key;
  if (key !== combo.key) return false;
  if (event.altKey) return false;
  if (Boolean(combo.mod) !== (event.metaKey || event.ctrlKey)) return false;
  if (combo.shift !== undefined && combo.shift !== event.shiftKey) return false;
  return true;
}

/** Which binding this keystroke is, if any. Only the global ones can match. */
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

/**
 * How this machine writes a shortcut.
 *
 * A label naming a key the keyboard does not have is worse than no label, so
 * the glyphs follow the platform even though the matching does not.
 */
export function formatCombo(combo: Combo): string {
  const parts: string[] = [];
  if (combo.mod) parts.push(IS_MAC ? "⌘" : "Ctrl");
  if (combo.shift) parts.push(IS_MAC ? "⇧" : "Shift");
  parts.push(NAMED[combo.key] ?? combo.key.toUpperCase());
  return IS_MAC ? parts.join("") : parts.join("+");
}

/** The arrow pair reads as one shortcut, so the panel draws it as one. */
export function comboLabel(binding: Binding): string {
  if (binding.id === "mention" || binding.id === "hit")
    return formatCombo({ key: "ArrowUp" }) + formatCombo({ key: "ArrowDown" });
  if (binding.id === "scope")
    return `${formatCombo({ key: "Tab" })} / ${formatCombo({ key: "Tab", shift: true })}`;
  return formatCombo(binding.combo);
}
