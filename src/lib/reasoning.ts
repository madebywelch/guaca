/**
 * What a turn shows of its own thinking.
 *
 * A model that publishes its working produces it as fast as it writes an
 * answer, and none of it is kept: the runtime addresses it to the placeholder
 * rather than to a channel, and the store drops it when the stream ends. What
 * is decided here is how much of it is held while it runs, and how the one line
 * that is drawn without being asked for is read.
 *
 * The whole turn is held, because the operator can ask to see it. What is drawn
 * is not the tail of it. A tail replaced sixty times a second says a turn is
 * alive and nothing else, which is what this was: through a wait that can run
 * to ten minutes it is a flicker nobody can read a word of. Two things make it
 * legible, and neither invents anything the model did not write.
 *
 * - **The heading is the model's own.** Reasoning arrives as prose with section
 *   headings in it, and a heading changes every half minute rather than every
 *   frame. It is the label, so the eye has something to hold on to and the
 *   answer to "what is it doing" survives longer than a glance.
 * - **The line under it is the last sentence that finished.** Nobody reads a
 *   sentence as it is typed. Waiting for the period costs a second or two of
 *   staleness and buys a line that can actually be read.
 *
 * Everything else is behind the disclosure, where it is the model's working as
 * written and this file has no opinion about it.
 */

/**
 * Characters of thinking held per agent.
 *
 * A ceiling, not a budget. A turn that reasons for ten minutes writes tens of
 * kilobytes and all of it is worth having behind the disclosure, but a stream
 * that never ends must not grow the window without bound. The head goes first,
 * because the panel follows the end and the newest words are the ones being
 * waited on.
 */
const KEPT = 200_000;

/**
 * How far back the drawn line is looked for.
 *
 * The heading and the newest finished sentence are always within a paragraph or
 * two of the end, and this is recomputed every time a delta lands. Scanning a
 * whole turn's thinking sixty times a second to find the last period in it
 * is work nobody can see.
 */
const TAIL = 4000;

/** Characters of that line drawn at once. */
const SHOWN = 110;

/** A line that is nothing but a section heading, in either convention. */
const HEADING = /^(?:#{1,6}\s+(.+)|\*\*([^*]+)\*\*)$/;

/**
 * Where a sentence ended.
 *
 * A terminator followed by a space, a newline, or nothing more yet. The last of
 * those is what lets a sentence settle the moment it is finished rather than
 * when the next one starts, and its price is a decimal point at the very end of
 * the buffer reading as a period for one frame.
 */
const ENDS = /[.!?](?=\s|$)/g;

/** What the one line of chrome draws. */
export interface Thought {
  /** The model's own section heading, where it has published one. */
  heading: string;
  /** The last sentence it finished under that heading. */
  line: string;
}

/**
 * Adds what just arrived, keeping the end.
 *
 * Raw, newlines and all: the line boundaries are what say where a heading
 * starts and where a sentence was ended by something other than a period,
 * and a reduction that dropped them ran the end of one thought into the
 * beginning of the next.
 */
export function keepThought(held: string | undefined, arriving: string): string {
  const next = (held ?? "") + arriving;
  return next.length > KEPT ? next.slice(-KEPT) : next;
}

/**
 * The heading the model is working under, and the last sentence it finished.
 *
 * Either can be empty: a model that publishes no headings has only the line, a
 * turn that has just published one has only the heading, and a turn that has
 * said nothing yet has neither, which is what leaves the plain sentence about
 * the agent still working on screen.
 */
export function thoughtNow(held: string | undefined): Thought {
  const lines = recent(held ?? "").split("\n");
  // Whatever follows the last newline is still being written. Every line before
  // it was finished by the newline that ended it, whether or not it was ended
  // by a period as well.
  const open = (lines.pop() ?? "").trim();

  let heading = "";
  let line = "";

  for (const raw of lines) {
    const at = raw.trim();
    if (!at) continue;
    const title = headingOf(at);
    if (title !== null) {
      // A new heading is a new subject, so the sentence said under the last one
      // stops being what this turn is doing.
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
      // The sentence being typed is drawn only while there is nothing else to
      // draw, which is the first few seconds of a turn. After that a heading or
      // a finished sentence is on screen and replacing it every frame with half
      // of the next one is the flicker this exists to stop.
      if (settled) line = settled;
      else if (!line && !heading) line = open;
    }
  }

  return { heading: clip(plain(heading)), line: clip(plain(line)) };
}

/**
 * The end of what has been held, starting at a line boundary.
 *
 * A slice can land in the middle of a line, and half a line is not one: it can
 * read as a heading nobody wrote or as a sentence that starts mid-word. Where
 * the whole slice is one unbroken paragraph there is no boundary to find and it
 * is read as it is, which is the right answer for a paragraph that long.
 */
function recent(text: string): string {
  if (text.length <= TAIL) return text;
  const tail = text.slice(-TAIL);
  const start = tail.indexOf("\n");
  return start === -1 ? tail : tail.slice(start + 1);
}

/** The heading a line is, or null for prose. */
function headingOf(line: string): string | null {
  const found = HEADING.exec(line);
  return found ? (found[1] ?? found[2] ?? "").trim() : null;
}

/** The last sentence in a line that has finished, or nothing. */
function lastSentence(line: string): string {
  const ends = [...line.matchAll(ENDS)].map((found) => found.index + 1);
  if (ends.length === 0) return "";
  const from = ends.length > 1 ? ends[ends.length - 2]! : 0;
  return line.slice(from, ends[ends.length - 1]!).trim();
}

/**
 * The words, without the marks around them.
 *
 * This is a line of muted chrome, not a document, so `**` and backticks come
 * off rather than get rendered. Nothing else does: stripping every character
 * markdown can use would turn `update_notes` into `updatenotes`, which is a
 * tool the operator does not have.
 */
function plain(line: string): string {
  return line.replace(/\*\*/g, "").replace(/`/g, "").trim();
}

/** Cuts a line to what the chrome can hold, saying that it was cut. */
function clip(line: string): string {
  return line.length > SHOWN ? `${line.slice(0, SHOWN - 1)}…` : line;
}
