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

/** A stretch of a message body: prose, or one `@` that resolved to an agent. */
export type Run =
  | { kind: "text"; at: number; text: string }
  | { kind: "mention"; at: number; text: string; name: string };

/** Letters, digits and underscore, in any script an operator types in. */
const WORD = /[\p{L}\p{N}_]/u;

/**
 * Splits a body into prose and the mentions inside it.
 *
 * Resolution against the roster is the whole rule: `@Critic` is a mention
 * because there is a Critic, and `@lunch` is a word somebody wrote. Nothing
 * else can decide it, because `@` followed by a word is also a handle, a
 * decorator and half an email address, and drawing a chip around one of those
 * tells the operator this app knows something it does not.
 *
 * Runs carry the text as it was typed rather than the roster's spelling, which
 * matters in the composer: the layer this paints sits under the operator's own
 * characters, so a chip drawn one glyph wider than the name under it is a pill
 * that has slid off. `name` is the canonical one, for anything asking who was
 * meant.
 */
export function splitMentions(text: string, names: string[]): Run[] {
  if (!text) return [];
  // Longest first, because a roster can hold both "Head" and "Head Chef": the
  // shorter one matches first otherwise and the rest of the name reads as prose.
  const roster = names.filter((name) => name.length > 0).sort((a, b) => b.length - a.length);
  if (roster.length === 0) return [{ kind: "text", at: 0, text }];

  const runs: Run[] = [];
  let plain = 0;
  let from = 0;

  while (from < text.length) {
    const found = text.indexOf("@", from);
    if (found === -1) break;
    const name = resolve(text, found, roster);
    if (!name) {
      from = found + 1;
      continue;
    }
    if (found > plain) runs.push({ kind: "text", at: plain, text: text.slice(plain, found) });
    const end = found + 1 + name.length;
    runs.push({ kind: "mention", at: found, text: text.slice(found, end), name });
    from = end;
    plain = end;
  }

  if (plain < text.length) runs.push({ kind: "text", at: plain, text: text.slice(plain) });
  return runs;
}

/** The agent an `@` at this index names, or null if it names nobody. */
function resolve(text: string, at: number, roster: string[]): string | null {
  // The same rule the typeahead opens on, so what lights up is what could have
  // been completed: an `@` mid-word is an email address, not a mention.
  const preceding = at === 0 ? "" : text[at - 1]!;
  if (preceding && !/\s|[([{]/.test(preceding)) return null;

  const rest = text.slice(at + 1);
  const lower = rest.toLowerCase();
  for (const name of roster) {
    if (!lower.startsWith(name.toLowerCase())) continue;
    // "Critic" must not light up inside "@Critical". Punctuation after a name
    // is ordinary prose and ends it.
    const after = rest[name.length];
    if (after !== undefined && WORD.test(after)) continue;
    return name;
  }
  return null;
}
