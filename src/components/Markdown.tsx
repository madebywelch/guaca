import { type ComponentProps, createContext, type ReactNode, useContext, useMemo } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

import { LABEL, readCallout } from "../lib/callout";
import { fenceLanguage, fenceText, readFigure } from "../lib/figure";
import { openExternal } from "../lib/ipc";
import { splitMentions } from "../lib/mentions";
import { Figure } from "./Figure";

export const Roster = createContext<string[]>([]);

export function Markdown({ children, live = false }: { children: string; live?: boolean }) {
  const names = useContext(Roster);
  const plugins = useMemo(() => [remarkGfm, remarkCallouts, remarkMentions(names)], [names]);
  const components = useMemo<NonNullable<ComponentProps<typeof ReactMarkdown>["components"]>>(
    () => ({
      a: ({ href, children }) => (
        <a
          href={href}
          onClick={(event) => {
            event.preventDefault();
            if (href) void openExternal(href);
          }}
        >
          {children}
        </a>
      ),
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
    }),
    [live],
  );

  return (
    <div className="md">
      <ReactMarkdown remarkPlugins={plugins} components={components}>
        {children}
      </ReactMarkdown>
    </div>
  );
}

function asFence(children: ReactNode): { language: string; source: string } | null {
  const only = Array.isArray(children) ? children[0] : children;
  if (!only || typeof only !== "object" || !("props" in only)) return null;
  const props = (only as { props?: { className?: unknown; children?: unknown } }).props;
  if (!props) return null;
  const language = fenceLanguage(props.className);
  if (!language) return null;
  return { language, source: fenceText(props.children) };
}

interface Node {
  type: string;
  value?: string;
  children?: Node[];
  data?: Record<string, unknown>;
}

const NOT_PROSE = new Set(["link", "linkReference"]);

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
