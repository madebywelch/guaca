import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
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

function files(dir, extension) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return files(path, extension);
    return extension.test(path) && !/\.test\.tsx?$/.test(path) ? [path] : [];
  });
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
}
