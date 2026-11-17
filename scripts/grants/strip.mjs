import { spawnSync } from "node:child_process";
import { parseSync } from "rolldown/utils";
import { lineEnd, lineStarts } from "./lex.mjs";

const HORIZONTAL = /[ \t]/;
const OPENERS = /[{([]\s*$/;
const CLOSERS = /^\s*[})\]]/;

function emptyJsxContainers(text, dialect) {
  if (dialect !== "tsx" && dialect !== "jsx") return [];
  const result = parseSync("file." + dialect, text, { lang: dialect, sourceType: "module" });
  const found = [];
  const visit = (node) => {
    if (node.type === "JSXExpressionContainer" && node.expression?.type === "JSXEmptyExpression") found.push([node.start, node.end]);
    for (const value of Object.values(node)) {
      if (Array.isArray(value)) {
        for (const child of value) if (child && typeof child.type === "string") visit(child);
      } else if (value && typeof value === "object" && typeof value.type === "string") visit(value);
    }
  };
  visit(result.program);
  return found;
}

function rangeOf(text, from, to) {
  const lineStart = text.lastIndexOf("\n", from - 1) + 1;
  const end = lineEnd(text, to);
  const ownLine = text.slice(lineStart, from).trim() === "";
  const restClean = text.slice(to, end).trim() === "";
  if (ownLine && restClean) return { from: lineStart, to: Math.min(end + 1, text.length), wholeLines: true };
  if (restClean) {
    let start = from;
    while (start > lineStart && HORIZONTAL.test(text[start - 1])) start--;
    return { from: start, to: end, wholeLines: false };
  }
  let after = to;
  while (after < end && HORIZONTAL.test(text[after])) after++;
  if (ownLine) return { from, to: after, wholeLines: false };
  const spaceBefore = from > lineStart && HORIZONTAL.test(text[from - 1]);
  return { from, to: spaceBefore ? after : to, wholeLines: false };
}

function collapse(lines, at) {
  const blank = (i) => i >= 0 && i < lines.length && lines[i].trim() === "";
  const before = at - 1;
  if (before < 0) {
    while (lines.length > 1 && blank(0)) lines.splice(0, 1);
    return;
  }
  if (blank(at) && ( blank(before) || OPENERS.test(lines[before]))) {
    lines.splice(at, 1);
    return;
  }
  if (blank(before) && (at >= lines.length || CLOSERS.test(lines[at]) || (at === lines.length - 1 && lines[at] === ""))) {
    lines.splice(before, 1);
  }
}

export function stripComments(text, units, { dialect } = {}) {
  const targets = units.filter((unit) => unit.kind === "comment");
  if (targets.length === 0) return { text, removed: [] };
  const containers = emptyJsxContainers(text, dialect);
  const ranges = targets
    .map((unit) => {
      const box = containers.find(([start, end]) => start <= unit.start && unit.end <= end);
      return rangeOf(text, box ? box[0] : unit.start, box ? box[1] : unit.end);
    })
    .sort((x, y) => x.from - y.from);
  const merged = [];
  for (const range of ranges) {
    const last = merged[merged.length - 1];
    if (last && range.from <= last.to) {
      last.to = Math.max(last.to, range.to);
      last.wholeLines = last.wholeLines && range.wholeLines;
    } else merged.push({ ...range });
  }
  let out = "";
  let cursor = 0;
  const points = [];
  for (const range of merged) {
    out += text.slice(cursor, range.from);
    if (range.wholeLines) points.push(out.length);
    cursor = range.to;
  }
  out += text.slice(cursor);
  const lines = out.split("\n");
  const starts = lineStarts(out);
  const at = (offset) => {
    let line = 0;
    while (line + 1 < starts.length && starts[line + 1] <= offset) line++;
    return line;
  };
  const pointLines = [...new Set(points.map(at))].sort((a, b) => b - a);
  for (const line of pointLines) collapse(lines, line);
  let result = lines.join("\n");
  if (text.endsWith("\n") && result !== "" && !result.endsWith("\n")) result += "\n";
  return { text: result, removed: targets };
}

const PYTHON_FILL = [
  "import sys",
  "lines = sys.stdin.read().split('\\n')",
  "while True:",
  "    try:",
  "        compile('\\n'.join(lines), 'file', 'exec')",
  "        break",
  "    except SyntaxError as error:",
  "        if 'expected an indented block' not in str(error):",
  "            raise",
  "        opener = error.lineno - 2",
  "        while lines[opener].strip() == '':",
  "            opener -= 1",
  "        while not lines[opener].rstrip().endswith(':'):",
  "            opener -= 1",
  "        indent = len(lines[opener]) - len(lines[opener].lstrip()) + 4",
  "        lines.insert(opener + 1, ' ' * indent + 'pass')",
  "sys.stdout.write('\\n'.join(lines))",
].join("\n");

export function fillEmptyPythonBodies(text) {
  const run = spawnSync("python3", ["-I", "-c", PYTHON_FILL], { input: text, encoding: "utf8", maxBuffer: 1 << 28 });
  if (run.status !== 0) throw new Error(run.stderr.trim().split("\n").pop());
  return run.stdout;
}
