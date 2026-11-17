export type Register = "asks" | "aside";

const MARKERS = new Map<string, Register>([
  ["important", "asks"],
  ["warning", "asks"],
  ["caution", "asks"],
  ["note", "aside"],
  ["tip", "aside"],
]);

export const LABEL: Record<Register, string> = { asks: "Needs you", aside: "Note" };

const MARKER = /^\[!([a-z]+)\][^\S\n]*(?:\n|$)/i;

export function readCallout(opening: string): { register: Register; rest: string } | null {
  const found = MARKER.exec(opening);
  if (!found) return null;

  const register = MARKERS.get(found[1]!.toLowerCase());
  if (!register) return null;

  return { register, rest: opening.slice(found[0].length) };
}
