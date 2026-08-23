import type { ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { fenceLanguage, fenceText, readFigure } from "../lib/figure";
import { openExternal } from "../lib/ipc";
import { Figure } from "./Figure";

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
 */
export function Markdown({ children }: { children: string }) {
  return (
    <div className="md">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
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
              const figure = readFigure(fenced.language, fenced.source);
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
