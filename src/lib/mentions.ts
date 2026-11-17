export interface MentionQuery {
  start: number;
  end: number;
  term: string;
}

export function mentionAt(text: string, caret: number): MentionQuery | null {
  if (caret < 0 || caret > text.length) return null;

  const before = text.slice(0, caret);
  const at = before.lastIndexOf("@");
  if (at === -1) return null;

  const preceding = at === 0 ? "" : before[at - 1]!;
  if (preceding && !/\s|[([{]/.test(preceding)) return null;

  const term = before.slice(at + 1);
  if (/\n/.test(term)) return null;
  if ((term.match(/ /g)?.length ?? 0) > 2) return null;
  if (term.length > 32) return null;

  return { start: at, end: caret, term };
}

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

export function applyMention(
  text: string,
  query: MentionQuery,
  name: string,
): { text: string; caret: number } {
  const inserted = `@${name} `;
  const next = text.slice(0, query.start) + inserted + text.slice(query.end);
  return { text: next, caret: query.start + inserted.length };
}

export type Run =
  | { kind: "text"; at: number; text: string }
  | { kind: "mention"; at: number; text: string; name: string };

const WORD = /[\p{L}\p{N}_]/u;

export function splitMentions(text: string, names: string[]): Run[] {
  if (!text) return [];
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

function resolve(text: string, at: number, roster: string[]): string | null {
  const preceding = at === 0 ? "" : text[at - 1]!;
  if (preceding && !/\s|[([{]/.test(preceding)) return null;

  const rest = text.slice(at + 1);
  const lower = rest.toLowerCase();
  for (const name of roster) {
    if (!lower.startsWith(name.toLowerCase())) continue;
    const after = rest[name.length];
    if (after !== undefined && WORD.test(after)) continue;
    return name;
  }
  return null;
}
