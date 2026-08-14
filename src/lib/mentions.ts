/**
 * `@` mention parsing for the composer.
 *
 * Exact agent names matter more here than in a normal chat app: `send_message`
 * resolves recipients by name, so a typo is a message that never arrives. The
 * typeahead exists to make the exact name the easy thing to type.
 */

export interface MentionQuery {
  /** Index of the `@`. */
  start: number;
  /** Index just past the caret. */
  end: number;
  /** What has been typed after the `@`, possibly empty. */
  term: string;
}

/**
 * Finds an in-progress mention immediately before the caret.
 *
 * Returns null when the caret is not in one, which includes the case of an `@`
 * in the middle of a word such as an email address.
 */
export function mentionAt(text: string, caret: number): MentionQuery | null {
  if (caret < 0 || caret > text.length) return null;

  const before = text.slice(0, caret);
  const at = before.lastIndexOf("@");
  if (at === -1) return null;

  // Must start a word, otherwise "someone@example.com" opens a menu.
  const preceding = at === 0 ? "" : before[at - 1]!;
  if (preceding && !/\s|[([{]/.test(preceding)) return null;

  const term = before.slice(at + 1);
  // Agent names can contain a space or two ("Head Sous Chef"), but past that
  // the `@` was several words ago and the operator has moved on. The caller
  // narrows further by hiding the menu when nothing matches.
  if (/\n/.test(term)) return null;
  if ((term.match(/ /g)?.length ?? 0) > 2) return null;
  if (term.length > 32) return null;

  return { start: at, end: caret, term };
}

/** Names matching what has been typed, best first. */
export function matchMentions(names: string[], term: string, limit = 6): string[] {
  const needle = term.trim().toLowerCase();
  if (!needle) return names.slice(0, limit);

  const starts: string[] = [];
  const contains: string[] = [];
  for (const name of names) {
    const haystack = name.toLowerCase();
    if (haystack.startsWith(needle)) starts.push(name);
    else if (haystack.includes(needle)) contains.push(name);
  }
  return [...starts, ...contains].slice(0, limit);
}

/** Replaces the in-progress mention with a complete one. */
export function applyMention(
  text: string,
  query: MentionQuery,
  name: string,
): { text: string; caret: number } {
  const inserted = `@${name} `;
  const next = text.slice(0, query.start) + inserted + text.slice(query.end);
  return { text: next, caret: query.start + inserted.length };
}
