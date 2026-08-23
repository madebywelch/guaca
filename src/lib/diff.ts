/**
 * Two versions of one small document, as the lines between them.
 *
 * There is one caller: an agent rewriting its own memory. The tool replaces the
 * file rather than appending to it, which is the right interface for a model
 * and the wrong one for whoever has to read the result: every write arrives as
 * the whole page, and "what did it decide to remember this time" means holding
 * two near-identical pages in your head and comparing them by eye.
 *
 * Line-level and nothing finer. A memory is a page of markdown written in
 * sentences, so a line is the unit an agent changes and the unit a person
 * reads; picking out the three words inside a rewritten sentence would draw
 * attention to the smallest part of the smallest change.
 *
 * Every unchanged line is kept, unlike a patch, which folds them away. A page
 * is short enough to show whole, and showing it whole answers the other
 * question the operator has when they open this: not only what changed, but
 * what the agent now believes.
 */

export type DiffKind = "same" | "added" | "removed";

export interface DiffLine {
  kind: DiffKind;
  text: string;
}

/**
 * Beyond this many cells the table is not worth building.
 *
 * The exact diff costs one cell per pair of lines. Memory is capped at a page
 * by `MAX_NOTES`, so the ordinary case is tens of lines against tens of lines
 * and nowhere near this; a document that reaches it is one where a line-by-line
 * comparison was never going to be read anyway, and it is shown as a wholesale
 * replacement rather than left to lock up the window working out otherwise.
 */
const CELLS = 250_000;

/**
 * Lines, with the empty document having none rather than one blank one.
 *
 * `"".split("\n")` is `[""]`, and one empty line is what makes a first memory
 * read as having replaced a blank line that was never there.
 */
function lines(value: string): string[] {
  return value.length === 0 ? [] : value.split(/\r?\n/);
}

/**
 * What changed between two versions, in order.
 *
 * Removals come before additions within a run, which is the arrangement a
 * rewritten line wants: the old sentence and the new one end up adjacent
 * instead of separated by everything else the turn touched.
 */
export function lineDiff(before: string, after: string): DiffLine[] {
  const old = lines(before);
  const now = lines(after);

  // Matching ends first. It is what makes the ordinary case cheap — a page
  // rewritten with one sentence changed is two short middles — and it also
  // stops the table from being built at all when nothing moved.
  let head = 0;
  while (head < old.length && head < now.length && old[head] === now[head]) head++;

  let tail = 0;
  while (
    tail < old.length - head &&
    tail < now.length - head &&
    old[old.length - 1 - tail] === now[now.length - 1 - tail]
  ) {
    tail++;
  }

  const gone = old.slice(head, old.length - tail);
  const fresh = now.slice(head, now.length - tail);

  const out: DiffLine[] = old.slice(0, head).map((text) => ({ kind: "same", text }) as DiffLine);
  out.push(...middle(gone, fresh));
  out.push(...old.slice(old.length - tail).map((text) => ({ kind: "same", text }) as DiffLine));
  return out;
}

/** The part that actually differs, matched line for line where it can be. */
function middle(gone: string[], fresh: string[]): DiffLine[] {
  if (gone.length === 0 || fresh.length === 0 || gone.length * fresh.length > CELLS) {
    return [
      ...gone.map((text) => ({ kind: "removed", text }) as DiffLine),
      ...fresh.map((text) => ({ kind: "added", text }) as DiffLine),
    ];
  }

  // The longest run of lines the two versions still share, counted from the
  // end back so the walk forward can follow the larger number.
  const wide = fresh.length + 1;
  const kept = new Uint32Array((gone.length + 1) * wide);
  for (let i = gone.length - 1; i >= 0; i--) {
    for (let j = fresh.length - 1; j >= 0; j--) {
      kept[i * wide + j] =
        gone[i] === fresh[j]
          ? kept[(i + 1) * wide + j + 1]! + 1
          : Math.max(kept[(i + 1) * wide + j]!, kept[i * wide + j + 1]!);
    }
  }

  const out: DiffLine[] = [];
  // Held back so a rewritten line reads as the old one and then the new one,
  // rather than as whatever order the walk happened to take them in.
  let added: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < gone.length && j < fresh.length) {
    if (gone[i] === fresh[j]) {
      out.push(...added, { kind: "same", text: gone[i]! });
      added = [];
      i++;
      j++;
    } else if (kept[(i + 1) * wide + j]! >= kept[i * wide + j + 1]!) {
      out.push({ kind: "removed", text: gone[i]! });
      i++;
    } else {
      added.push({ kind: "added", text: fresh[j]! });
      j++;
    }
  }
  out.push(...gone.slice(i).map((text) => ({ kind: "removed", text }) as DiffLine));
  out.push(...added);
  out.push(...fresh.slice(j).map((text) => ({ kind: "added", text }) as DiffLine));
  return out;
}

/** How much of it changed, which is the part that goes next to the title. */
export function diffTally(diff: DiffLine[]): { added: number; removed: number } {
  return {
    added: diff.filter((line) => line.kind === "added").length,
    removed: diff.filter((line) => line.kind === "removed").length,
  };
}

/**
 * The tally in words.
 *
 * Words rather than `+3 −1` because the rest of a channel is written in them,
 * and because a plus sign is a symbol a screen reader may not say at all: this
 * sentence is the only part of a diff that reads as anything out loud.
 */
export function diffSummary(diff: DiffLine[]): string {
  const { added, removed } = diffTally(diff);
  const count = (n: number, word: string) => `${n} line${n === 1 ? "" : "s"} ${word}`;
  if (added > 0 && removed > 0) return `${count(added, "added")}, ${removed} removed`;
  if (added > 0) return count(added, "added");
  if (removed > 0) return count(removed, "removed");
  return "rewritten unchanged";
}
