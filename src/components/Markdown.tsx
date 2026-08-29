import { createContext, type ReactNode, useContext, useMemo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { LABEL, readCallout } from "../lib/callout";
import { fenceLanguage, fenceText, readFigure } from "../lib/figure";
import { openExternal } from "../lib/ipc";
import { splitMentions } from "../lib/mentions";
import { Figure } from "./Figure";

/**
 * Every name an `@` in a body is allowed to resolve to.
 *
 * A context rather than a store subscription, and the difference is a hundred
 * of them: a body is drawn once per message, and this is read by every one of
 * them on every render. It is also what keeps the default honest. Nothing
 * resolves against an empty roster, so a surface that has not provided one
 * draws exactly the prose the model wrote instead of guessing at names.
 */
export const Roster = createContext<string[]>([]);

/**
 * Renders message bodies as Markdown.
 *
 * Models write Markdown whether or not you ask them to, so plain text means
 * reading raw `**bold**` and unformatted lists all day.
 *
 * Raw HTML is deliberately not enabled, and a fenced `html` block is not a
 * hole in that. Message bodies come from a model, and on the peer path from
 * *another agent's* model, which is the least trustworthy input in the app, so
 * markup in the prose is ignored: `react-markdown` drops embedded HTML unless
 * `rehype-raw` is added, and it is not. A fence tagged as a page is a different
 * thing entirely. It never becomes part of this document, and is run on an
 * origin of its own that can reach nothing. `artifact.rs` is the argument.
 *
 * `live` says this body is still being written, and only one caller sets it:
 * the streaming bubble. It reaches exactly one decision, in {@link readFigure},
 * which is whether a page is framed now or when the reply settles.
 */
export function Markdown({ children, live = false }: { children: string; live?: boolean }) {
  const names = useContext(Roster);
  const plugins = useMemo(() => [remarkGfm, remarkCallouts, remarkMentions(names)], [names]);

  return (
    <div className="md">
      <ReactMarkdown
        remarkPlugins={plugins}
        components={{
          a: ({ href, children }) => (
            <a
              href={href}
              onClick={(event) => {
                // Following a link inside the webview navigates away from the
                // app with no way back.
                event.preventDefault();
                if (href) void openExternal(href);
              }}
            >
              {children}
            </a>
          ),
          // Remote images cannot load under the app's content policy, so a
          // plain image would render as a broken box. A link says what it is.
          img: ({ src, alt }) => (
            <a
              href={typeof src === "string" ? src : undefined}
              onClick={(event) => {
                event.preventDefault();
                if (typeof src === "string") void openExternal(src);
              }}
            >
              {alt || "image"}
            </a>
          ),
          // Intercepted at the `pre` rather than at the `code` inside it: a
          // figure is a block, and a block returned from the `code` slot lands
          // inside a `<pre>` that react-markdown has already opened, which is
          // markup no browser agrees on how to lay out.
          pre: ({ children }) => {
            const fenced = asFence(children);
            if (fenced) {
              const figure = readFigure(fenced.language, fenced.source, live);
              if (figure.kind !== "source") {
                return <Figure figure={figure} source={fenced.source} />;
              }
            }
            return <pre className="md__pre">{children}</pre>;
          },
          table: ({ children }) => (
            <div className="md__scroll">
              <table>{children}</table>
            </div>
          ),
        }}
      >
        {children}
      </ReactMarkdown>
    </div>
  );
}

/**
 * The language and text of a fenced block, given the node `pre` was handed.
 *
 * A `pre` holds exactly one `code` element when it came from a fence, and
 * something else entirely when it came from an indented block. Only the first
 * is a figure: an indented block carries no language, so there is nothing to
 * decide from and nothing to draw.
 */
function asFence(children: ReactNode): { language: string; source: string } | null {
  const only = Array.isArray(children) ? children[0] : children;
  if (!only || typeof only !== "object" || !("props" in only)) return null;
  const props = (only as { props?: { className?: unknown; children?: unknown } }).props;
  if (!props) return null;
  const language = fenceLanguage(props.className);
  if (!language) return null;
  return { language, source: fenceText(props.children) };
}

/**
 * mdast, in the shape these walks need and no more.
 *
 * The real types live in `@types/mdast`, which is not a dependency of this app:
 * it arrives under `react-markdown` and would be a version nothing here pins.
 * A tree walk needs three fields and a name for them.
 */
interface Node {
  type: string;
  value?: string;
  children?: Node[];
  data?: Record<string, unknown>;
}

/**
 * Node types whose text is not prose, so an `@` inside one is left alone.
 *
 * Code and fences never reach the walk at all, because they carry a value
 * rather than text children. A link does have text children, and a chip inside
 * an anchor is two things claiming the same click; `remark-gfm` also autolinks
 * a bare email address, which is the one place an `@` is guaranteed not to be
 * a mention.
 */
const NOT_PROSE = new Set(["link", "linkReference"]);

/**
 * Wraps every resolved `@` in its own element, before anything is rendered.
 *
 * A remark plugin rather than a pass over the rendered output: a mention can
 * appear inside a bold run, a list item, a heading or a table cell, and the
 * tree is the one place all of those are the same thing. The chip is a `span`
 * with a class, declared through `hName` and `hProperties`, so the document
 * `react-markdown` builds is still plain hast and the rule about raw HTML is
 * untouched.
 */
function remarkMentions(names: string[]) {
  return () => (tree: Node) => {
    if (names.length > 0) mark(tree, names);
  };
}

function mark(node: Node, names: string[]): void {
  if (!node.children || NOT_PROSE.has(node.type)) return;

  const next: Node[] = [];
  for (const child of node.children) {
    if (child.type !== "text" || typeof child.value !== "string") {
      mark(child, names);
      next.push(child);
      continue;
    }

    const runs = splitMentions(child.value, names);
    // The common answer: nothing in this text names anybody, so the node it
    // arrived as is the node that goes back.
    if (!runs.some((run) => run.kind === "mention")) {
      next.push(child);
      continue;
    }

    for (const run of runs) {
      next.push(
        run.kind === "text"
          ? { type: "text", value: run.text }
          : {
              type: "mention",
              data: {
                hName: "span",
                hProperties: { className: "mention", "data-mention": run.name },
              },
              children: [{ type: "text", value: run.text }],
            },
      );
    }
  }

  node.children = next;
}

/**
 * Draws a quote that opens with an alert marker as a box, before anything is
 * rendered.
 *
 * A remark plugin for the reason {@link remarkMentions} is one: a callout
 * holds prose, and prose is a list, a link, a name, a table and a line of
 * code, which the tree is the one place all of are the same thing.
 *
 * It stops being a `blockquote` on the way out, because it is not a quote.
 * `hName` is a `div`, so every rule under `.md blockquote` misses it and no
 * landmark is opened in the middle of a message; the label goes in as the
 * first child, so a screen reader reads *Needs you* in the place a sighted
 * operator sees the box, with no ARIA holding the two together.
 */
function remarkCallouts() {
  return (tree: Node) => box(tree);
}

function box(node: Node): void {
  if (!node.children) return;
  if (node.type === "blockquote") open(node);
  for (const child of node.children) box(child);
}

function open(quote: Node): void {
  const first = quote.children?.[0];
  if (first?.type !== "paragraph") return;
  const opening = first.children?.[0];
  if (opening?.type !== "text" || typeof opening.value !== "string") return;

  const found = readCallout(opening.value);
  if (!found) return;

  opening.value = found.rest;
  // A marker on a line of its own leaves an empty paragraph where it was,
  // which draws as a blank line between the label and the first thing said.
  if (found.rest === "" && first.children?.length === 1) quote.children?.shift();

  quote.data = {
    hName: "div",
    hProperties: { className: `callout callout--${found.register}` },
  };
  quote.children?.unshift({
    type: "calloutLabel",
    data: { hName: "p", hProperties: { className: "callout__label" } },
    children: [{ type: "text", value: LABEL[found.register] }],
  });
}
