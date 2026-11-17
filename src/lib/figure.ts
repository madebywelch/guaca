import { type ChartRead, readChart } from "./chart";

export type Figure =
  | { kind: "chart"; read: ChartRead; source: string }
  | { kind: "html"; html: string }
  | { kind: "pending" }
  | { kind: "source" };

const CHART = new Set(["chart", "graph", "plot"]);
const PAGE = new Set(["html", "artifact"]);

const LEAST_PAGE = 40;

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

export function readFigure(language: string, source: string, live = false): Figure {
  const tag = language.trim().toLowerCase();
  const body = source.trim();

  if (PAGE.has(tag) && live) return { kind: "pending" };

  if (body.length === 0) return { kind: "source" };

  if (CHART.has(tag)) {
    if (!looksComplete(body)) return { kind: "pending" };
    let value: unknown;
    try {
      value = JSON.parse(body);
    } catch {
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

export function fenceLanguage(className: unknown): string {
  if (typeof className !== "string") return "";
  return (
    className
      .split(/\s+/)
      .find((name) => name.startsWith("language-"))
      ?.slice(9) ?? ""
  );
}

export function fenceText(children: unknown): string {
  if (typeof children === "string") return children;
  if (Array.isArray(children)) return children.map(fenceText).join("");
  return "";
}
