export function questions(count: number): string {
  return count === 1 ? "1 question" : `${count} questions`;
}

export function results(count: number): string {
  return count === 1 ? "1 result" : `${count} results`;
}

export const DELETED = "A deleted agent";

const NAMED = 3;

export function whoIsWaiting(asked: string[]): string {
  const gone = asked.filter((name) => name === DELETED).length;
  const first = asked.indexOf(DELETED);
  const kept = asked.filter((name) => name !== DELETED);
  const names =
    gone < 2 ? asked : [...kept.slice(0, first), `${gone} deleted agents`, ...kept.slice(first)];
  if (names.length === 0) return "Nobody is waiting on you";
  if (names.length === 1) return `${names[0]} ${gone < 2 ? "is" : "are"} waiting on you`;
  const others = names.length - (NAMED - 1);
  const shown = (
    names.length <= NAMED
      ? names
      : [...names.slice(0, NAMED - 1), others === 1 ? "1 other" : `${others} others`]
  ).map((name, at) => (at > 0 && name === DELETED ? "a deleted agent" : name));
  return `${shown.slice(0, -1).join(", ")} and ${shown.at(-1)} are waiting on you`;
}

export function askers(items: { agentId: string }[], nameOf: (id: string) => string): string[] {
  return [...new Set(items.map((one) => one.agentId))].map(nameOf);
}
