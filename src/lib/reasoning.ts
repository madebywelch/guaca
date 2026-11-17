const KEPT = 200_000;

const TAIL = 4000;

const SHOWN = 110;

const HEADING = /^(?:#{1,6}\s+(.+)|\*\*([^*]+)\*\*)$/;

const ENDS = /[.!?](?=\s|$)/g;

export interface Thought {
  heading: string;
  line: string;
}

export function keepThought(held: string | undefined, arriving: string): string {
  const next = (held ?? "") + arriving;
  return next.length > KEPT ? next.slice(-KEPT) : next;
}

export function thoughtNow(held: string | undefined): Thought {
  const lines = recent(held ?? "").split("\n");
  const open = (lines.pop() ?? "").trim();

  let heading = "";
  let line = "";

  for (const raw of lines) {
    const at = raw.trim();
    if (!at) continue;
    const title = headingOf(at);
    if (title !== null) {
      heading = title;
      line = "";
      continue;
    }
    line = lastSentence(at) || at;
  }

  if (open) {
    const title = headingOf(open);
    if (title !== null) {
      heading = title;
      line = "";
    } else {
      const settled = lastSentence(open);
      if (settled) line = settled;
      else if (!line && !heading) line = open;
    }
  }

  return { heading: clip(plain(heading)), line: clip(plain(line)) };
}

function recent(text: string): string {
  if (text.length <= TAIL) return text;
  const tail = text.slice(-TAIL);
  const start = tail.indexOf("\n");
  return start === -1 ? tail : tail.slice(start + 1);
}

function headingOf(line: string): string | null {
  const found = HEADING.exec(line);
  return found ? (found[1] ?? found[2] ?? "").trim() : null;
}

function lastSentence(line: string): string {
  const ends = [...line.matchAll(ENDS)].map((found) => found.index + 1);
  if (ends.length === 0) return "";
  const from = ends.length > 1 ? ends[ends.length - 2]! : 0;
  return line.slice(from, ends[ends.length - 1]!).trim();
}

function plain(line: string): string {
  return line.replace(/\*\*/g, "").replace(/`/g, "").trim();
}

function clip(line: string): string {
  return line.length > SHOWN ? `${line.slice(0, SHOWN - 1)}…` : line;
}
