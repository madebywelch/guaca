export type DiffKind = "same" | "added" | "removed";

export interface DiffLine {
  kind: DiffKind;
  text: string;
}

const CELLS = 250_000;

function lines(value: string): string[] {
  return value.length === 0 ? [] : value.split(/\r?\n/);
}

export function lineDiff(before: string, after: string): DiffLine[] {
  const old = lines(before);
  const now = lines(after);

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

function middle(gone: string[], fresh: string[]): DiffLine[] {
  if (gone.length === 0 || fresh.length === 0 || gone.length * fresh.length > CELLS) {
    return [
      ...gone.map((text) => ({ kind: "removed", text }) as DiffLine),
      ...fresh.map((text) => ({ kind: "added", text }) as DiffLine),
    ];
  }

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

export function diffTally(diff: DiffLine[]): { added: number; removed: number } {
  return {
    added: diff.filter((line) => line.kind === "added").length,
    removed: diff.filter((line) => line.kind === "removed").length,
  };
}

export function diffSummary(diff: DiffLine[]): string {
  const { added, removed } = diffTally(diff);
  const count = (n: number, word: string) => `${n} line${n === 1 ? "" : "s"} ${word}`;
  if (added > 0 && removed > 0) return `${count(added, "added")}, ${removed} removed`;
  if (added > 0) return count(added, "added");
  if (removed > 0) return count(removed, "removed");
  return "rewritten unchanged";
}
