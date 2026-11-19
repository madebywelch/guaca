import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { parseSync } from "vite";
import allowed from "./operator-copy-agent-strings.mjs";
export function frontendDashes(source) {
  const lines = [];
  let block = false;
  let quote = "";
  let escaped = false;
  for (const [index, line] of source.split("\n").entries()) {
    for (let i = 0; i < line.length; i++) {
      const char = line[i];
      const next = line[i + 1];
      if (block) {
        if (char === "*" && next === "/") { block = false; i++; }
      } else if (quote) {
        if (char === "\u2014") lines.push(index + 1);
        if (escaped) escaped = false;
        else if (char === "\\") escaped = true;
        else if (char === quote) quote = "";
      } else if (char === "/" && next === "*") {
        block = true;
        i++;
      } else if (char === "/" && next === "/") {
        break;
      } else if (char === "\"" || char === "'" || char === "`") {
        quote = char;
      } else if (char === "\u2014") {
        lines.push(index + 1);
      }
    }
  }
  return [...new Set(lines)];
}

const NOUN = /\b(items?|turns?|decisions?|requests?)\b/i;
const WAITING = /\b(waiting on you|needs? you|needs? your (attention|answer)|For you|reminders?)\b/i;
const RETIRED = [
  [/\b(items?|turns?|decisions?|requests?) (is |are )?(waiting|needs?)\b/i, "a waiting request is a question"],
  [/\bfollow(ing)?[ -]through\b/i, "say \"In progress after your answer\""],
  [/\bteam activity\b/i, "say \"When each agent last worked\""],
  [/\bFor You\b/, "the place is \"For you\""],
  [/\b(open|whose) (a |the )?channel\b|\bthe channel with\b/i, "an agent's page is its conversation"],
];

export function retiredWords(text) {
  const found = RETIRED.filter(([pattern]) => pattern.test(text)).map(([, fix]) => fix);
  if (NOUN.test(text) && WAITING.test(text)) found.push("a waiting request is a question, finished work is a result");
  return [...new Set(found)];
}

function lineAt(source, offset) {
  return source.slice(0, offset).split("\n").length;
}

export function frontendCopy(source, path) {
  const lang = path.endsWith(".tsx") ? "tsx" : "ts";
  const units = [];
  const visit = (node, unit) => {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) {
      for (const child of node) visit(child, unit);
      return;
    }
    if (node.type === "ImportDeclaration" || node.type === "ExportAllDeclaration") return;
    if (node.type === "ExportNamedDeclaration") {
      visit(node.declaration, unit);
      return;
    }
    let inside = unit;
    const opens = node.type === "JSXElement" || node.type === "JSXFragment" || (node.type === "TemplateLiteral" && !unit?.template);
    if (opens) {
      inside = { line: lineAt(source, node.start), parts: [], template: unit?.template || node.type === "TemplateLiteral" };
      units.push(inside);
    }
    const text =
      node.type === "JSXText" ? node.value
      : node.type === "TemplateElement" ? node.value.cooked ?? node.value.raw
      : node.type === "Literal" && typeof node.value === "string" ? node.value
      : null;
    if (text !== null) {
      if (inside) inside.parts.push(text);
      else units.push({ line: lineAt(source, node.start), parts: [text] });
    }
    for (const [key, value] of Object.entries(node)) {
      if (key !== "type" && key !== "start" && key !== "end") visit(value, inside);
    }
  };
  visit(parseSync(path, source, { lang }).program, null);
  return units.map((unit) => ({ line: unit.line, text: unit.parts.join(" ").replace(/\s+/g, " ") }));
}

export const RUST_COPY = [
  "src-tauri/src/menubar.rs",
  "src-tauri/src/domain/outbox.rs",
  "src-tauri/src/push.rs",
  "src-tauri/src/notifications.rs",
  "src-tauri/src/db/decisions.rs",
];

export function rustCopy(source) {
  return source.split("\n").flatMap((line, index) => {
    if (/^\s*\/\//.test(line)) return [];
    const strings = line.match(/"(?:[^"\\]|\\.)*"/g);
    return strings ? [{ line: index + 1, text: strings.map((one) => one.slice(1, -1)).join(" ") }] : [];
  });
}

function files(dir, extension) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return files(path, extension);
    return extension.test(path) && !/\.test\.tsx?$/.test(path) ? [path] : [];
  });
}

export function vocabulary() {
  const errors = [];
  const report = (path, copy) => {
    for (const { line, text } of copy) {
      for (const fix of retiredWords(text)) errors.push(`${path}:${line}: ${fix}: "${text.trim()}"`);
    }
  };
  for (const path of files("src", /\.tsx?$/)) report(path, frontendCopy(readFileSync(path, "utf8"), path));
  for (const path of RUST_COPY) {
    if (existsSync(path)) report(path, rustCopy(readFileSync(path, "utf8")));
    else errors.push(`${path}: listed in RUST_COPY but missing; point the list at the file that holds this copy now`);
  }
  return errors;
}

export function check() {
  const errors = [];
  for (const path of files("src", /\.(ts|tsx|css)$/)) {
    for (const line of frontendDashes(readFileSync(path, "utf8"), path)) {
      errors.push(`${path}:${line}`);
    }
  }
  for (const path of files("src-tauri/src", /\.rs$/)) {
    const lines = readFileSync(path, "utf8").split("\n");
    for (const [index, line] of lines.entries()) {
      if (!/^\s*(\/\/|\*|--)/.test(line) && line.includes("\u2014") && !allowed[path]?.includes(line.trim())) {
        errors.push(`${path}:${index + 1}`);
      }
    }
  }
  return errors;
}

if (process.argv[1]?.endsWith("check-operator-copy.mjs")) {
  const errors = check();
  if (errors.length) {
    console.error(`Em dashes in operator copy:\n${errors.join("\n")}`);
    process.exitCode = 1;
  }
  const words = vocabulary();
  if (words.length) {
    console.error(`Retired words for a waiting request in operator copy:\n${words.join("\n")}`);
    process.exitCode = 1;
  }
}
