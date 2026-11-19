export function questions(count: number): string {
  return count === 1 ? "1 question" : `${count} questions`;
}

export function results(count: number): string {
  return count === 1 ? "1 result" : `${count} results`;
}

const NAMED = 3;

export function whoIsWaiting(names: string[]): string {
  const unique = [...new Set(names)];
  if (unique.length === 0) return "Nobody is waiting on you";
  if (unique.length === 1) return `${unique[0]} is waiting on you`;
  const others = unique.length - (NAMED - 1);
  const shown =
    unique.length <= NAMED
      ? unique
      : [...unique.slice(0, NAMED - 1), others === 1 ? "1 other" : `${others} others`];
  return `${shown.slice(0, -1).join(", ")} and ${shown.at(-1)} are waiting on you`;
}
