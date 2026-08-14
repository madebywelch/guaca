import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { openExternal } from "../lib/ipc";

/**
 * Renders message bodies as Markdown.
 *
 * Models write Markdown whether or not you ask them to, so plain text means
 * reading raw `**bold**` and unformatted lists all day.
 *
 * Raw HTML is deliberately not enabled. Message bodies come from a model, and
 * on the peer path from *another agent's* model, which is the least trustworthy
 * input in the app. `react-markdown` ignores embedded HTML unless `rehype-raw`
 * is added, and it is not.
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
          pre: ({ children }) => <pre className="md__pre">{children}</pre>,
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
