/**
 * A fenced block the transcript draws instead of printing.
 *
 * An agent asked for last quarter by region has the numbers in hand at the
 * moment it writes the sentence about them, so the cheapest possible way to
 * get a chart out of it is to let it write one where it is already writing.
 * A fence costs no extra model call, streams in with the rest of the reply,
 * and needs nothing new in the runtime: `as_plain_text` already returns the
 * text of a message, so the record, the prompt, the dedup fingerprint, search,
 * and a peer's copy of the reply all keep working with no change at all. A
 * tool call for the same thing would be a round trip to send back data the
 * model had already finished computing.
 *
 * It also means the agent can read back what it drew. A figure that lived
 * outside the text would be invisible to the turn after it, and an agent that
 * cannot see its own last chart draws it again.
 *
 * Three languages are figures and everything else is source, which is the rule
 * that keeps ```python from turning into something nobody asked for.
 */

import { type ChartRead, readChart } from "./chart";

/**
 * What a fence turns out to be.
 *
 * `pending` is the shape that matters most and is the least obvious. A reply
 * arrives a token at a time, so a chart spends most of its life on screen as
 * half a JSON object. Drawn as an error, that is a red box that appears under
 * every figure for a second and then goes away, which teaches an operator that
 * the feature is broken. Drawn as `pending`, it is a figure that has not
 * finished arriving, which is what it is.
 */
export type Figure =
  | { kind: "chart"; read: ChartRead; source: string }
  | { kind: "html"; html: string }
  | { kind: "pending" }
  | { kind: "source" };

/** The languages that mean something other than "show me this text". */
const CHART = new Set(["chart", "graph", "plot"]);
const PAGE = new Set(["html", "artifact"]);

/**
 * An HTML document has to be worth the frame it costs.
 *
 * A one-line fence is a snippet an operator wants to read, not a page they
 * want to look at, and framing `<div>hi</div>` on its own origin is a renderer
 * and a network round trip spent on something a code block already showed.
 */
const LEAST_PAGE = 40;

/**
 * Whether a JSON document has finished arriving.
 *
 * Cheaper and more honest than trying to parse: a spec that is still streaming
 * has an unclosed brace, and one that has closed every brace is as complete as
 * it is ever going to be, whether or not it parses. Quoted braces do not count,
 * and a backslash inside a string escapes the character after it, so neither
 * `{"label": "}"}` nor `{"label": "\""}` is read as finished early.
 */
export function looksComplete(source: string): boolean {
  let depth = 0;
  let inString = false;
  let escaped = false;
  let opened = false;

  for (const character of source) {
    if (escaped) {
      escaped = false;
      continue;
    }
    if (inString) {
      if (character === "\\") escaped = true;
      else if (character === '"') inString = false;
      continue;
    }
    if (character === '"') inString = true;
    else if (character === "{" || character === "[") {
      depth++;
      opened = true;
    } else if (character === "}" || character === "]") depth--;
  }

  return opened && depth === 0 && !inString;
}

/**
 * Reads a fence, given the language it was tagged with.
 *
 * Undecidable cases come back as `source`, never as an error. A model that
 * writes a chart the app cannot draw has still written something the operator
 * can read, and showing them the text is strictly better than showing them a
 * refusal where the text used to be. What a *valid* refusal looks like is the
 * chart's own business: {@link readChart} returns the sentence, and the figure
 * draws it under the source so the agent can be told what to change.
 *
 * `live` is whether the message this fence is in is still being written, and
 * it separates the two figures rather than gating both. A chart is drawn from
 * a value by a pure function, so redrawing it every token is what makes one
 * assemble itself on screen and costs nothing. A page is drawn by registering
 * a document and pointing a frame at the address that comes back, so the same
 * treatment is a reload per token: a round trip each, a new entry each in a
 * store that holds two dozen, and a frame that throws away whatever the
 * operator had done in it every sixteen milliseconds.
 */
export function readFigure(language: string, source: string, live = false): Figure {
  const tag = language.trim().toLowerCase();
  const body = source.trim();

  // Before the emptiness check, so a page announces itself the moment its
  // fence opens instead of flashing an empty code block first.
  if (PAGE.has(tag) && live) return { kind: "pending" };

  if (body.length === 0) return { kind: "source" };

  if (CHART.has(tag)) {
    if (!looksComplete(body)) return { kind: "pending" };
    let value: unknown;
    try {
      value = JSON.parse(body);
    } catch {
      // Every brace closed and it still will not parse, so this is not a chart
      // that is on its way: it is one that was written wrongly.
      return {
        kind: "chart",
        read: { why: "This is not valid JSON, so there is nothing to draw yet." },
        source: body,
      };
    }
    return { kind: "chart", read: readChart(value), source: body };
  }

  if (PAGE.has(tag) && body.length >= LEAST_PAGE && /<[a-z!]/i.test(body)) {
    return { kind: "html", html: body };
  }

  return { kind: "source" };
}

/**
 * The language a fenced block was tagged with, as react-markdown reports it.
 *
 * `language-json` on the `code` element, which is the only place the tag
 * survives: by the time it is a React node the info string is gone.
 */
export function fenceLanguage(className: unknown): string {
  if (typeof className !== "string") return "";
  return (
    className
      .split(/\s+/)
      .find((name) => name.startsWith("language-"))
      ?.slice(9) ?? ""
  );
}

/**
 * The text inside a fenced block.
 *
 * React children rather than a string, because the block has already been
 * turned into nodes. Anything that is not a string is skipped instead of
 * stringified: a fence holds text, and a node in there means the markdown was
 * something other than what this is looking for.
 */
export function fenceText(children: unknown): string {
  if (typeof children === "string") return children;
  if (Array.isArray(children)) return children.map(fenceText).join("");
  return "";
}
