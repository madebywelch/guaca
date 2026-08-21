/**
 * What a turn shows of its own thinking.
 *
 * A model that publishes its working produces it as fast as it writes an
 * answer, and none of it is kept: the runtime addresses it to the placeholder
 * rather than to a channel, and the store drops it when the stream ends. So the
 * only question here is how much of it is worth holding while it runs.
 *
 * The answer is a tail. A turn that reasons for two minutes writes tens of
 * kilobytes into a slice that is rewritten every sixteen milliseconds, and
 * nobody reads any of it except the last line: the one line under the composer
 * is there to say what is happening *now*. Keeping the head instead would
 * freeze that line on the first thing the model thought.
 */

/**
 * Raw reasoning held per agent. Comfortably more than one line, so the line
 * being written survives the paragraph before it being forgotten.
 */
const KEPT = 240;

/** Characters of that line drawn at once, before it starts scrolling. */
const SHOWN = 100;

/**
 * Adds what just arrived, keeping the end.
 *
 * Raw, newlines and all: the paragraph boundaries are what tell the line where
 * it starts, and a reduction that dropped them ran the end of one thought into
 * the beginning of the next.
 */
export function keepThought(held: string | undefined, arriving: string): string {
  const next = (held ?? "") + arriving;
  return next.length > KEPT ? next.slice(-KEPT) : next;
}

/**
 * The one line to draw, as the model would have written it.
 *
 * Reasoning arrives as prose with markdown section headings in it. This is a
 * single muted line of chrome, not a document, so the marks come off rather
 * than get rendered: `**Checking the totals**` is a heading everywhere except
 * here, where it is four characters of noise.
 */
export function thoughtLine(held: string | undefined): string {
  const lines = (held ?? "").split("\n").map(plain).filter(Boolean);
  const line = lines[lines.length - 1] ?? "";
  // Truncated from the front, so the newest words keep arriving on the right.
  // The other way round, the line stops changing the moment it is full.
  return line.length > SHOWN ? `…${line.slice(-SHOWN)}` : line;
}

function plain(line: string): string {
  return line
    .replace(/\*\*/g, "")
    .replace(/`/g, "")
    .replace(/^#{1,6}\s*/, "")
    .trim();
}
