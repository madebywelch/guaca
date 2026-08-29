/**
 * The one part of a reply that is addressed to the operator rather than read
 * by them.
 *
 * An agent that has been working for ten minutes writes nine paragraphs, and
 * one of them is the sentence that needs a person: a key only they can rotate,
 * a decision nobody else can take, a thing about to go out that should not.
 * Written as prose it is the fourth paragraph of nine, and the operator finds
 * it last or not at all. A box is what fixes that, and it is the cheapest fix
 * there is: the agent already knows which sentence it was as it writes it.
 *
 * The syntax is GitHub's alert marker, and that is the whole reason it is that
 * rather than something of ours. Models write `> [!IMPORTANT]` unprompted, so
 * this draws correctly on a reply written before the prompt mentioned it, on
 * one from an agent skimming its instructions, and on every transcript already
 * in the database. A marker this file does not know stays a quote with its own
 * words in it, which is the rule `figure.ts` keeps for a
 * fence it cannot draw.
 *
 * A quote and not a fence, because what goes in the box is prose: a list, a
 * link, a name, a table, a line of code. A fence holds text and could hold
 * none of them.
 */

/**
 * Which of the two boxes a marker opens.
 *
 * Two, because the app has one accent and spending it on decoration is how it
 * stops meaning "answer me". `asks` is that amber, and it says here what it
 * says everywhere else in Guaca: a person has to do something. `aside` is the
 * quiet box, for a thing worth seeing that needs nobody.
 */
export type Register = "asks" | "aside";

/** GitHub's five, sorted into the two registers this app can draw. */
const MARKERS = new Map<string, Register>([
  ["important", "asks"],
  ["warning", "asks"],
  ["caution", "asks"],
  ["note", "aside"],
  ["tip", "aside"],
]);

/**
 * The word over the box, which is the app's and never the model's.
 *
 * Five markers and two words. An agent that writes `[!CAUTION]` and one that
 * writes `[!WARNING]` mean the same thing here, and drawing both of their
 * words is an operator learning a vocabulary that decides nothing: what they
 * need to know is whether this box is for them. *Needs you* is what the rail
 * already says about an agent that is waiting on somebody, so the row in the
 * rail and the box in the transcript say one thing in one set of words.
 */
export const LABEL: Record<Register, string> = { asks: "Needs you", aside: "Note" };

/**
 * The marker, alone on the first line of the quote.
 *
 * Alone is load-bearing: `[!IMPORTANT] ship it` is a sentence that opens with
 * a bracket, and a box drawn round it would eat the two words after the
 * marker into a label nobody wrote. Trailing blanks are allowed because a
 * model leaves them and no operator can see one.
 */
const MARKER = /^\[!([a-z]+)\][^\S\n]*(?:\n|$)/i;

/**
 * Reads the opening text of a quote.
 *
 * `rest` is what is left of that text once the marker line is off it, which is
 * usually nothing: the marker gets its own line and the words start on the
 * next one. It is not always nothing, because a model that writes the whole
 * quote as one soft-wrapped paragraph is writing valid markdown and means the
 * same thing.
 */
export function readCallout(opening: string): { register: Register; rest: string } | null {
  const found = MARKER.exec(opening);
  if (!found) return null;

  const register = MARKERS.get(found[1]!.toLowerCase());
  if (!register) return null;

  return { register, rest: opening.slice(found[0].length) };
}
