import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { parseSync } from "rolldown/utils";

const PYTHON_SCANNER = fileURLToPath(new URL("./scan.py", import.meta.url));

const JS_DIALECTS = new Map([
  [".ts", "ts"],
  [".mts", "ts"],
  [".cts", "ts"],
  [".tsx", "tsx"],
  [".js", "js"],
  [".mjs", "js"],
  [".cjs", "js"],
  [".jsx", "jsx"],
]);

const HASH_LINE_FILES =
  /(^|\/)(\.gitignore|\.dockerignore|\.gitattributes|\.npmrc|\.editorconfig|\.env[^/]*|[^/]+\.(service|example|ini|conf))$/;

export function languageOf(path, text) {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 ? name.slice(dot).toLowerCase() : "";
  if (ext === ".rs") return { lang: "rust" };
  if (JS_DIALECTS.has(ext)) return { lang: "js", dialect: JS_DIALECTS.get(ext) };
  if (ext === ".py") return { lang: "python" };
  if (ext === ".sh" || ext === ".bash" || ext === ".zsh") return { lang: "shell" };
  if (ext === ".swift") return { lang: "swift" };
  if (ext === ".yml" || ext === ".yaml") return { lang: "yaml" };
  if (ext === ".toml") return { lang: "toml" };
  if (name === "Dockerfile" || name.startsWith("Dockerfile.") || ext === ".dockerfile") return { lang: "dockerfile" };
  if (ext === ".css") return { lang: "css" };
  if ([".html", ".htm", ".svg", ".xml"].includes(ext)) return { lang: "markup" };
  if (ext === ".sql") return { lang: "sql" };
  if (ext === ".tla") return { lang: "tla" };
  if (ext === ".json" || ext === ".jsonc" || ext === ".json5") return { lang: "jsonc" };
  if (HASH_LINE_FILES.test(path)) return { lang: "hash-lines" };
  if (ext === "" && text.startsWith("#!")) {
    const shebang = text.slice(0, lineEnd(text, 0));
    if (/\b(ba|z|da)?sh\b/.test(shebang)) return { lang: "shell" };
    if (/\bpython/.test(shebang)) return { lang: "python" };
    if (/\b(node|bun|deno)\b/.test(shebang)) return { lang: "js", dialect: "js" };
  }
  return null;
}

export function isTestOrScript(path, lang) {
  if (lang === "shell" || lang === "python" || lang === "swift") return true;
  return /(^|\/)(scripts|tests?|e2e|fixtures)\/|\.(test|spec)\.[^/]+$|_test\.[^/]+$/.test(path);
}

export function scanFiles(files) {
  const results = new Map();
  const python = [];
  for (const file of files) {
    const kind = languageOf(file.path, file.text);
    if (!kind) continue;
    const out = { lang: kind.lang, comments: [], marks: [], unsafeOffsets: [], error: null };
    results.set(file.path, out);
    const wait = isTestOrScript(file.path, kind.lang);
    try {
      if (kind.lang === "python") python.push({ text: file.text, wait, out });
      else LEXERS[kind.lang](file.text, out, { path: file.path, dialect: kind.dialect, wait, offset: 0 });
    } catch (error) {
      out.error = String(error.message ?? error);
    }
  }
  if (python.length > 0) scanPython(python);
  return results;
}

function scanPython(entries) {
  const run = spawnSync("python3", ["-I", PYTHON_SCANNER], {
    input: JSON.stringify(entries.map(({ text, wait }) => ({ text, wait }))),
    encoding: "utf8",
    maxBuffer: 1 << 28,
  });
  if (run.status !== 0) throw new Error("python3 " + PYTHON_SCANNER + " failed: " + run.stderr);
  const answers = JSON.parse(run.stdout);
  entries.forEach((entry, index) => {
    const answer = answers[index];
    if (answer.error) {
      entry.out.error = answer.error;
      return;
    }
    const starts = lineStarts(entry.text);
    for (const [line, col, endLine, endCol] of answer.comments) {
      entry.out.comments.push({ start: starts[line - 1] + col, end: starts[endLine - 1] + endCol });
    }
    for (const [line, endLine] of answer.strings) {
      const start = starts[line - 1];
      const first = entry.text.slice(start, lineEnd(entry.text, start));
      entry.out.comments.push({
        start: start + first.length - first.trimStart().length,
        end: lineEnd(entry.text, starts[endLine - 1]),
      });
    }
    for (const [line, what] of answer.marks) {
      const start = starts[line - 1];
      entry.out.marks.push({ start, end: lineEnd(entry.text, start), what });
    }
  });
}

export function lineStarts(text) {
  const starts = [0];
  for (let i = 0; i < text.length; i++) if (text.charCodeAt(i) === 10) starts.push(i + 1);
  return starts;
}

export function lineEnd(text, from) {
  const at = text.indexOf("\n", from);
  return at < 0 ? text.length : at;
}

const IDENT_START = /[\p{XID_Start}_]/u;
const IDENT = /[\p{XID_Start}_][\p{XID_Continue}]*/uy;

function matchAt(re, text, i) {
  re.lastIndex = i;
  return re.exec(text);
}

function skipQuoted(text, i, quote) {
  let j = i + 1;
  while (j < text.length && text[j] !== quote) j += text[j] === "\\" ? 2 : 1;
  return Math.min(j + 1, text.length);
}

function skipNestedBlock(text, i) {
  let depth = 1;
  let j = i + 2;
  while (j < text.length && depth > 0) {
    if (text[j] === "/" && text[j + 1] === "*") {
      depth++;
      j += 2;
    } else if (text[j] === "*" && text[j + 1] === "/") {
      depth--;
      j += 2;
    } else j++;
  }
  return j;
}

function lexRust(text, out, ctx) {
  const tokens = [];
  const n = text.length;
  const push = (t, s, e, v = text.slice(s, e)) => tokens.push({ t, v, s, e });
  let i = 0;
  if (text.startsWith("#!") && !/^#!\s*\[/.test(text)) {
    i = lineEnd(text, 0);
    out.comments.push({ start: 0, end: i });
  }
  while (i < n) {
    const c = text[i];
    if (c === " " || c === "\t" || c === "\n" || c === "\r") {
      i++;
    } else if (c === "/" && text[i + 1] === "/") {
      const end = lineEnd(text, i);
      out.comments.push({ start: i, end });
      i = end;
    } else if (c === "/" && text[i + 1] === "*") {
      const end = skipNestedBlock(text, i);
      out.comments.push({ start: i, end });
      i = end;
    } else if (c === '"') {
      const end = skipQuoted(text, i, '"');
      push("lit", i, end);
      i = end;
    } else if (c === "'") {
      let end = 0;
      if (text[i + 1] === "\\") {
        end = text.indexOf("'", i + 3) + 1;
      } else if (i + 1 < n) {
        const width = String.fromCodePoint(text.codePointAt(i + 1)).length;
        if (text[i + 1 + width] === "'") end = i + 2 + width;
      }
      if (end > 0) {
        push("lit", i, end);
        i = end;
      } else {
        const id = matchAt(IDENT, text, i + 1);
        const stop = id ? i + 1 + id[0].length : i + 1;
        push("lifetime", i, stop);
        i = stop;
      }
    } else if (IDENT_START.test(String.fromCodePoint(text.codePointAt(i)))) {
      const raw = matchAt(/(?:b|c)?r(#*)"/y, text, i);
      if (raw) {
        const close = '"' + raw[1];
        const at = text.indexOf(close, i + raw[0].length);
        const end = at < 0 ? n : at + close.length;
        push("lit", i, end);
        i = end;
      } else if ((c === "b" || c === "c") && text[i + 1] === '"') {
        const end = skipQuoted(text, i + 1, '"');
        push("lit", i, end);
        i = end;
      } else if (c === "b" && text[i + 1] === "'") {
        const end = text[i + 2] === "\\" ? text.indexOf("'", i + 4) + 1 : i + 4;
        push("lit", i, end);
        i = end;
      } else {
        const rawIdent = matchAt(/r#([\p{XID_Start}_][\p{XID_Continue}]*)/uy, text, i);
        const id = rawIdent ? rawIdent : matchAt(IDENT, text, i);
        push("ident", i, i + id[0].length, rawIdent ? rawIdent[1] : id[0]);
        i += id[0].length;
      }
    } else if (c >= "0" && c <= "9") {
      const num = matchAt(/[0-9]\w*(?:\.[0-9]\w*)?/y, text, i);
      push("lit", i, i + num[0].length);
      i += num[0].length;
    } else {
      push("punct", i, i + 1);
      i++;
    }
  }
  rustStructure(tokens, out, ctx);
}

function attributeSuppression(inner) {
  const names = inner.filter((token) => token.t === "ident").map((token) => token.v);
  const head = names[0];
  if (head === "allow" || head === "expect" || head === "ignore") return "#[" + head + "]";
  if (head === "rustfmt" && names[1] === "skip") return "#[rustfmt::skip]";
  if (head === "cfg_attr") {
    const found = names.slice(1).find((name) => name === "allow" || name === "expect" || name === "ignore");
    if (found) return "#[cfg_attr(.., " + found + ")]";
  }
  return null;
}

function isTestAttribute(inner) {
  const values = inner.map((token) => token.v);
  const call = values.indexOf("(");
  const path = call < 0 ? values : values.slice(0, call);
  return path[path.length - 1] === "test" || values.join("") === "cfg(test)";
}

function poll(counted, rechecks) {
  return { counted, rechecks, exits: false, clock: false, sleeps: [] };
}

function settle(region, out) {
  if ((region.counted || region.clock) && (region.rechecks || region.exits)) return;
  for (const sleep of region.sleeps) out.marks.push(sleep);
}

function innermost(stack) {
  return stack.findLast((entry) => entry !== null && typeof entry === "object");
}

function sleepIn(stack, out, mark) {
  const region = innermost(stack);
  if (region) region.sleeps.push(mark);
  else out.marks.push(mark);
}

function rustStructure(tokens, out, ctx) {
  const stack = [];
  let parens = 0;
  let pendingTest = null;
  let pendingLoop = null;
  let inImpl = false;
  for (let k = 0; k < tokens.length; k++) {
    const token = tokens[k];
    if (token.t === "punct" && token.v === "#") {
      const bang = tokens[k + 1]?.v === "!" ? 1 : 0;
      if (tokens[k + 1 + bang]?.v !== "[") continue;
      let depth = 0;
      let close = k + 1 + bang;
      for (; close < tokens.length; close++) {
        if (tokens[close].v === "[") depth++;
        else if (tokens[close].v === "]" && --depth === 0) break;
      }
      const inner = tokens.slice(k + 2 + bang, close);
      const what = attributeSuppression(inner);
      if (what) out.marks.push({ start: token.s, end: tokens[close]?.e ?? token.e, what });
      if (isTestAttribute(inner)) pendingTest = parens;
      k = close;
      continue;
    }
    if (token.t === "ident") {
      if (token.v === "unsafe") out.unsafeOffsets.push(token.s);
      if (token.v === "impl") inImpl = true;
      const loopWord = token.v === "loop" || token.v === "while" || token.v === "for";
      if (loopWord && !(token.v === "for" && (inImpl || tokens[k + 1]?.v === "<"))) {
        pendingLoop = { parens, region: poll(token.v === "for", token.v === "while" && tokens[k + 1]?.v !== "true") };
      }
      if (token.v === "break" || token.v === "return") {
        const region = innermost(stack);
        if (region) region.exits = true;
      }
      if (token.v === "Instant" || token.v === "elapsed" || token.v === "SystemTime") {
        const region = pendingLoop?.region ?? innermost(stack);
        if (region) region.clock = true;
      }
      if ((token.v === "sleep" || token.v === "sleep_until") && tokens[k + 1]?.v === "(") {
        const before = tokens[k - 1]?.v;
        if (before !== "." && before !== "fn" && (ctx.wait || stack.includes("test"))) {
          sleepIn(stack, out, { start: token.s, end: token.e, what: "sleep" });
        }
      }
      continue;
    }
    if (token.t !== "punct") continue;
    if (token.v === "(" || token.v === "[") parens++;
    else if (token.v === ")" || token.v === "]") parens--;
    else if (token.v === "{") {
      let kind = null;
      if (pendingLoop?.parens === parens) {
        kind = pendingLoop.region;
        pendingLoop = null;
      } else if (pendingTest === parens) {
        kind = "test";
        pendingTest = null;
      }
      inImpl = false;
      stack.push(kind);
    } else if (token.v === "}") {
      const closed = stack.pop();
      if (closed !== null && typeof closed === "object") settle(closed, out);
    } else if (token.v === ";") {
      inImpl = false;
      if (pendingTest === parens) pendingTest = null;
    }
  }
}

const TEST_FUNCTIONS = new Set(["it", "test", "describe", "suite", "bench", "context"]);
const TEST_MODIFIERS = new Set(["skip", "only", "todo", "skipIf", "runIf"]);
const TEST_FLAGS = new Set(["skip", "only", "todo"]);
const LOOP_NODES = new Set(["ForStatement", "ForInStatement", "ForOfStatement", "WhileStatement", "DoWhileStatement"]);

function memberChain(node) {
  const names = [];
  let at = node;
  while (at?.type === "MemberExpression") {
    if (at.computed) return [];
    names.unshift(at.property.name);
    at = at.object;
  }
  if (at?.type !== "Identifier") return [];
  names.unshift(at.name);
  return names;
}

function lexJs(text, out, ctx) {
  const lang = ctx.dialect ?? "js";
  const result = parseSync("file." + lang, text, { lang, sourceType: "module" });
  if (result.errors.length > 0) throw new Error(result.errors[0].message);
  for (const comment of result.comments) {
    out.comments.push({ start: ctx.offset + comment.start, end: ctx.offset + comment.end });
  }
  const mark = (node, what) => out.marks.push({ start: ctx.offset + node.start, end: ctx.offset + node.end, what });
  const ancestors = [];
  const visit = (node) => {
    if (node.type === "CallExpression") inspectCall(node, ancestors, ctx, mark, text);
    ancestors.push(node);
    for (const value of Object.values(node)) {
      if (Array.isArray(value)) {
        for (const child of value) if (child && typeof child.type === "string") visit(child);
      } else if (value && typeof value === "object" && typeof value.type === "string") visit(value);
    }
    ancestors.pop();
  };
  visit(result.program);
}

function inspectCall(node, ancestors, ctx, mark, text) {
  const chain = memberChain(node.callee);
  if (chain.length === 0) return;
  const [base, ...rest] = chain;
  const modifier = rest.find((name) => TEST_MODIFIERS.has(name));
  if (TEST_FUNCTIONS.has(base) && modifier) return mark(node.callee, "." + modifier);
  if (base === "t" && rest.length === 1 && (rest[0] === "skip" || rest[0] === "todo")) return mark(node.callee, "t." + rest[0]);
  const named = chain[chain.length - 1];
  if (TEST_FUNCTIONS.has(named) && chain.length <= 2) {
    for (const arg of node.arguments) {
      if (arg.type !== "ObjectExpression") continue;
      const flag = arg.properties.find((p) => TEST_FLAGS.has(p.key?.name ?? p.key?.value));
      if (flag) return mark(flag, "{ " + (flag.key.name ?? flag.key.value) + " }");
    }
  }
  if (!ctx.wait || boundedPoll(ancestors)) return;
  if (named === "sleep" || named === "delay") return mark(node, "sleep");
  if (named !== "setTimeout") return;
  if (ancestors[ancestors.length - 1]?.type === "AwaitExpression") return mark(node, "sleep");
  const promise = ancestors.findLast(
    (a) => a.type === "NewExpression" && a.callee.type === "Identifier" && a.callee.name === "Promise",
  );
  const resolve = promise?.arguments[0]?.params?.[0];
  const callback = node.arguments[0];
  if (resolve?.type !== "Identifier" || !callback) return;
  const source = text.slice(callback.start, callback.end);
  if (source === resolve.name || new RegExp("\\b" + resolve.name + "\\s*\\(").test(source)) mark(node, "sleep");
}

const FUNCTION_NODES = new Set(["FunctionDeclaration", "FunctionExpression", "ArrowFunctionExpression"]);

function boundedPoll(ancestors) {
  const loop = ancestors.findLast((a) => LOOP_NODES.has(a.type));
  if (!loop) return false;
  const counted = loop.type === "ForOfStatement" || loop.type === "ForInStatement" || (loop.type === "ForStatement" && loop.test);
  const conditional = (loop.type === "WhileStatement" || loop.type === "DoWhileStatement") && loop.test.type !== "Literal";
  return Boolean(counted || readsClock(loop)) && (conditional || exitsLoop(loop.body));
}

function readsClock(node) {
  if (!node || typeof node.type !== "string" || FUNCTION_NODES.has(node.type)) return false;
  if (node.type === "CallExpression") {
    const chain = memberChain(node.callee).join(".");
    if (chain === "Date.now" || chain === "performance.now") return true;
  }
  if (node.type === "NewExpression" && node.callee.type === "Identifier" && node.callee.name === "Date") return true;
  return Object.values(node).some((value) =>
    Array.isArray(value) ? value.some((child) => readsClock(child)) : value && typeof value === "object" && readsClock(value),
  );
}

function exitsLoop(node) {
  if (!node || typeof node.type !== "string" || FUNCTION_NODES.has(node.type) || LOOP_NODES.has(node.type)) return false;
  if (node.type === "BreakStatement" || node.type === "ReturnStatement") return true;
  return Object.values(node).some((value) =>
    Array.isArray(value) ? value.some((child) => exitsLoop(child)) : value && typeof value === "object" && exitsLoop(value),
  );
}

const KEEPS_COMMAND_POSITION = new Set(["then", "do", "else", "elif", "if", "while", "until", "!", "{", "time", "exec", "command", "env"]);

function lexShell(text, out, ctx) {
  const n = text.length;
  const base = ctx.offset;
  let i = 0;
  let wordStart = true;
  let commandPosition = true;
  const loops = [];
  let pendingCounted = null;
  let heredocs = [];
  let word = null;
  const finishWord = () => {
    if (!word) return;
    const value = text.slice(word.s, i);
    if (word.command) {
      if (value === "for" || value === "while" || value === "until") {
        pendingCounted = poll(value === "for", value !== "for");
        pendingCounted.first = true;
      } else if (value === "do") {
        if (pendingCounted) loops.push(pendingCounted);
        pendingCounted = null;
      } else if (value === "done") {
        const closed = loops.pop();
        if (closed) settle(closed, out);
      } else if ((value === "break" || value === "return" || value === "exit") && loops.length > 0) {
        loops[loops.length - 1].exits = true;
      } else if (value === "sleep" && ctx.wait) sleepIn(loops, out, { start: base + word.s, end: base + i, what: "sleep" });
      if (pendingCounted?.first && value !== "for" && value !== "while" && value !== "until") {
        if (value === "true" || value === ":") pendingCounted.rechecks = false;
        pendingCounted.first = false;
      }
    }
    if (/\bSECONDS\b/.test(value) || (word.command && value === "date")) {
      const region = pendingCounted ?? loops[loops.length - 1];
      if (region) region.clock = true;
    }
    commandPosition = word.command && (KEEPS_COMMAND_POSITION.has(value) || /^[A-Za-z_]\w*=/.test(value));
    word = null;
  };
  const startWord = () => {
    if (!word) word = { s: i, command: commandPosition };
    wordStart = false;
  };
  while (i < n) {
    const c = text[i];
    if (c === "\n") {
      finishWord();
      i++;
      for (const delimiter of heredocs) {
        while (i < n) {
          const end = lineEnd(text, i);
          const line = text.slice(i, end);
          i = Math.min(end + 1, n);
          if (line.trim() === delimiter) break;
        }
      }
      heredocs = [];
      wordStart = true;
      commandPosition = true;
    } else if (c === " " || c === "\t" || c === "\r") {
      finishWord();
      i++;
      wordStart = true;
    } else if (c === "#" && wordStart) {
      finishWord();
      const end = lineEnd(text, i);
      out.comments.push({ start: base + i, end: base + end });
      i = end;
    } else if (c === "\\" && text[i + 1] === "\n") {
      finishWord();
      i += 2;
      wordStart = true;
    } else if (c === "\\") {
      startWord();
      i += 2;
    } else if (c === "'") {
      startWord();
      const end = text.indexOf("'", i + 1);
      i = end < 0 ? n : end + 1;
    } else if (c === '"') {
      startWord();
      i = skipShellDouble(text, i);
    } else if (c === "\u0060") {
      startWord();
      const end = text.indexOf("\u0060", i + 1);
      i = end < 0 ? n : end + 1;
    } else if (c === "$" && text[i + 1] === "'") {
      startWord();
      i = skipQuoted(text, i + 1, "'");
    } else if (c === "$" && text[i + 1] === "{") {
      startWord();
      i = skipBalanced(text, i + 1, "{", "}");
    } else if (c === "$" && text[i + 1] === "(" && text[i + 2] === "(") {
      startWord();
      i = skipBalanced(text, i + 1, "(", ")");
    } else if (c === "$" && text[i + 1] === "(") {
      finishWord();
      i += 2;
      wordStart = true;
      commandPosition = true;
    } else if (c === "<" && text[i + 1] === "<" && text[i + 2] !== "<") {
      finishWord();
      const doc = matchAt(/<<-?\s*(['"]?)([A-Za-z0-9_.-]+)\1/y, text, i);
      if (doc) {
        heredocs.push(doc[2]);
        i += doc[0].length;
      } else i += 2;
      wordStart = true;
    } else if (";&|()<>".includes(c)) {
      finishWord();
      const operator = matchAt(/;;&?|;&|&&|\|\||\|&|<<<|[;&|()<>]/y, text, i)[0];
      i += operator.length;
      wordStart = true;
      commandPosition = !"<>".includes(operator[0]);
    } else {
      startWord();
      i++;
    }
  }
  finishWord();
}

function skipShellDouble(text, i) {
  let j = i + 1;
  while (j < text.length && text[j] !== '"') {
    if (text[j] === "\\") j += 2;
    else if (text[j] === "$" && text[j + 1] === "(") j = skipBalanced(text, j + 1, "(", ")");
    else if (text[j] === "$" && text[j + 1] === "{") j = skipBalanced(text, j + 1, "{", "}");
    else j++;
  }
  return Math.min(j + 1, text.length);
}

function skipBalanced(text, i, open, close) {
  let depth = 0;
  let j = i;
  while (j < text.length) {
    const c = text[j];
    if (c === "\\") j += 2;
    else if (c === "'" && open === "(") j = text.indexOf("'", j + 1) + 1 || text.length;
    else if (c === '"') j = skipShellDouble(text, j);
    else {
      if (c === open) depth++;
      else if (c === close && --depth === 0) return j + 1;
      j++;
    }
  }
  return text.length;
}

function lexSwift(text, out, ctx) {
  const n = text.length;
  const stack = [];
  let pendingLoop = null;
  let i = 0;
  if (text.startsWith("#!")) {
    i = lineEnd(text, 0);
    out.comments.push({ start: 0, end: i });
  }
  while (i < n) {
    const c = text[i];
    if (c === "/" && text[i + 1] === "/") {
      const end = lineEnd(text, i);
      out.comments.push({ start: i, end });
      i = end;
    } else if (c === "/" && text[i + 1] === "*") {
      const end = skipNestedBlock(text, i);
      out.comments.push({ start: i, end });
      i = end;
    } else if (c === '"' || matchAt(/#+"/y, text, i)) {
      i = skipSwiftString(text, i);
    } else if (IDENT_START.test(c)) {
      const id = matchAt(IDENT, text, i)[0];
      if (id === "while" || id === "for" || id === "repeat") {
        pendingLoop = poll(id === "for", id === "while" && !matchAt(/\s+true\b/y, text, i + id.length));
      }
      if (id === "break" || id === "return") {
        const region = innermost(stack);
        if (region) region.exits = true;
      }
      if (id === "Date" || id === "DispatchTime" || id === "ContinuousClock" || id === "CFAbsoluteTimeGetCurrent") {
        const region = pendingLoop ?? innermost(stack);
        if (region) region.clock = true;
      }
      const call = matchAt(/\s*\(/y, text, i + id.length);
      if ((id === "sleep" || id === "usleep" || id === "nanosleep") && call && ctx.wait) {
        sleepIn(stack, out, { start: i, end: i + id.length, what: "sleep" });
      }
      i += id.length;
    } else {
      if (c === "{") {
        stack.push(pendingLoop);
        pendingLoop = null;
      } else if (c === "}") {
        const closed = stack.pop();
        if (closed) settle(closed, out);
      }
      i++;
    }
  }
}

function skipSwiftString(text, i) {
  const hashes = matchAt(/#*/y, text, i)[0];
  let j = i + hashes.length;
  const multi = text.startsWith('"""', j);
  const close = (multi ? '"""' : '"') + hashes;
  j += multi ? 3 : 1;
  while (j < text.length) {
    if (text.startsWith(close, j)) return j + close.length;
    if (text[j] === "\\" && text.startsWith(hashes, j + 1)) {
      const at = j + 1 + hashes.length;
      j = text[at] === "(" ? skipSwiftInterpolation(text, at) : at + 1;
    } else j++;
  }
  return text.length;
}

function skipSwiftInterpolation(text, i) {
  let depth = 0;
  let j = i;
  while (j < text.length) {
    const c = text[j];
    if (c === '"') {
      j = skipSwiftString(text, j);
      continue;
    }
    if (c === "(") depth++;
    else if (c === ")" && --depth === 0) return j + 1;
    j++;
  }
  return text.length;
}

function lexYaml(text, out, ctx) {
  let block = null;
  let quote = null;
  const flushRun = (until) => {
    if (block?.run) lexShell(text.slice(block.from, until), out, { ...ctx, offset: ctx.offset + block.from, wait: true });
    block = null;
  };
  for (const start of lineStarts(text)) {
    const end = lineEnd(text, start);
    const line = text.slice(start, end);
    const indent = line.length - line.trimStart().length;
    if (block) {
      if (line.trim() === "" || indent > block.indent) continue;
      flushRun(start);
    }
    let comment = -1;
    for (let j = 0; j < line.length; j++) {
      const c = line[j];
      if (quote) {
        if (quote === "'" && c === "'" && line[j + 1] === "'") j++;
        else if (quote === '"' && c === "\\") j++;
        else if (c === quote) quote = null;
      } else if (c === "#" && (j === 0 || line[j - 1] === " " || line[j - 1] === "\t")) {
        comment = j;
        break;
      } else if ((c === "'" || c === '"') && /(^|[:\-[{,?])\s*$/.test(line.slice(0, j))) quote = c;
    }
    if (comment >= 0) out.comments.push({ start: ctx.offset + start + comment, end: ctx.offset + end });
    const content = comment >= 0 ? line.slice(0, comment) : line;
    if (!quote && /(^|:|-)\s*[|>][-+0-9]*\s*$/.test(content)) {
      block = {
        indent,
        from: Math.min(end + 1, text.length),
        run: /(^|\s|-\s*)run:\s*[|>][-+0-9]*\s*$/.test(content),
      };
    }
  }
  flushRun(text.length);
}

function lexToml(text, out, ctx) {
  const n = text.length;
  let i = 0;
  let inLints = false;
  let lineIsHeader = false;
  let atLineStart = true;
  while (i < n) {
    const c = text[i];
    if (c === "\n") {
      atLineStart = true;
      lineIsHeader = false;
      i++;
      continue;
    }
    if (c === " " || c === "\t" || c === "\r") {
      i++;
      continue;
    }
    if (atLineStart && c === "[") {
      const header = text.slice(i, lineEnd(text, i));
      inLints = /^\[\[?\s*(workspace\.)?lints(\.|\s*\])/.test(header);
      lineIsHeader = true;
    }
    atLineStart = false;
    if (c === "#") {
      const end = lineEnd(text, i);
      out.comments.push({ start: ctx.offset + i, end: ctx.offset + end });
      i = end;
    } else if (c === '"' || c === "'") {
      const triple = c.repeat(3);
      let end;
      if (text.startsWith(triple, i)) {
        const at = text.indexOf(triple, i + 3);
        end = at < 0 ? n : at + 3;
        while (text[end] === c) end++;
      } else end = c === '"' ? skipQuoted(text, i, '"') : text.indexOf("'", i + 1) + 1 || n;
      if (inLints && !lineIsHeader && text.slice(i + 1, end - 1) === "allow") {
        out.marks.push({ start: ctx.offset + i, end: ctx.offset + end, what: "lints allow" });
      }
      i = end;
    } else i++;
  }
}

function lexDockerfile(text, out, ctx) {
  let instruction = null;
  for (const start of lineStarts(text)) {
    const end = lineEnd(text, start);
    const line = text.slice(start, end);
    if (line.trim().startsWith("#")) {
      out.comments.push({ start: start + line.indexOf("#"), end });
      continue;
    }
    if (!instruction) {
      const head = matchAt(/\s*(RUN|CMD|ENTRYPOINT)\s+(?!\[)/iy, text, start);
      instruction = { shell: Boolean(head), from: head ? start + head[0].length : start };
    }
    if (line.trimEnd().endsWith("\\")) continue;
    if (instruction.shell) {
      const segment = text.slice(instruction.from, end).replace(/\n\s*#[^\n]*/g, (m) => " ".repeat(m.length));
      lexShell(segment, out, { ...ctx, offset: ctx.offset + instruction.from, wait: true });
    }
    instruction = null;
  }
}

function lexCss(text, out, ctx) {
  const n = text.length;
  let i = 0;
  while (i < n) {
    const c = text[i];
    if (c === "/" && text[i + 1] === "*") {
      const at = text.indexOf("*/", i + 2);
      const end = at < 0 ? n : at + 2;
      out.comments.push({ start: ctx.offset + i, end: ctx.offset + end });
      i = end;
    } else if (c === '"' || c === "'") i = skipQuoted(text, i, c);
    else if (matchAt(/url\(\s*[^'"\s)]/iy, text, i)) i = text.indexOf(")", i) + 1 || n;
    else i++;
  }
}

function lexMarkup(text, out, ctx) {
  const n = text.length;
  let i = 0;
  while (i < n) {
    if (text.startsWith("<!--", i)) {
      const at = text.indexOf("-->", i + 4);
      const end = at < 0 ? n : at + 3;
      out.comments.push({ start: i, end });
      i = end;
    } else if (text.startsWith("<![CDATA[", i)) {
      const at = text.indexOf("]]>", i);
      i = at < 0 ? n : at + 3;
    } else if (text[i] === "<" && /[A-Za-z]/.test(text[i + 1] ?? "")) {
      const close = skipTag(text, i);
      const name = matchAt(/<([A-Za-z][\w:-]*)/y, text, i)[1].toLowerCase();
      const tag = text.slice(i, close);
      i = close;
      if (name !== "script" && name !== "style") continue;
      const endAt = text.toLowerCase().indexOf("</" + name, i);
      const bodyEnd = endAt < 0 ? n : endAt;
      const body = text.slice(i, bodyEnd);
      const type = /\btype\s*=\s*["']?([^"'\s>]+)/i.exec(tag)?.[1]?.toLowerCase();
      if (name === "style") lexCss(body, out, { ...ctx, offset: i });
      else if (!type || /javascript|module|ecmascript/.test(type)) lexJs(body, out, { ...ctx, dialect: "js", offset: i, wait: false });
      else if (type.includes("json")) lexJsonc(body, out, { ...ctx, offset: i });
      i = bodyEnd;
    } else i++;
  }
}

function skipTag(text, i) {
  let j = i + 1;
  while (j < text.length && text[j] !== ">") {
    if (text[j] === '"' || text[j] === "'") j = text.indexOf(text[j], j + 1) + 1 || text.length;
    else j++;
  }
  return j + 1;
}

function lexSql(text, out) {
  const n = text.length;
  let i = 0;
  while (i < n) {
    const c = text[i];
    if (c === "-" && text[i + 1] === "-") {
      const end = lineEnd(text, i);
      out.comments.push({ start: i, end });
      i = end;
    } else if (c === "/" && text[i + 1] === "*") {
      const at = text.indexOf("*/", i + 2);
      const end = at < 0 ? n : at + 2;
      out.comments.push({ start: i, end });
      i = end;
    } else if (c === "'" || c === '"' || c === "\u0060") {
      let j = i + 1;
      while (j < n && !(text[j] === c && text[j + 1] !== c)) j += text[j] === c ? 2 : 1;
      i = j + 1;
    } else if (c === "[") i = text.indexOf("]", i) + 1 || n;
    else i++;
  }
}

function lexJsonc(text, out, ctx) {
  const n = text.length;
  let i = 0;
  while (i < n) {
    const c = text[i];
    if (c === "/" && text[i + 1] === "/") {
      const end = lineEnd(text, i);
      out.comments.push({ start: ctx.offset + i, end: ctx.offset + end });
      i = end;
    } else if (c === "/" && text[i + 1] === "*") {
      const at = text.indexOf("*/", i + 2);
      const end = at < 0 ? n : at + 2;
      out.comments.push({ start: ctx.offset + i, end: ctx.offset + end });
      i = end;
    } else if (c === '"') i = skipQuoted(text, i, '"');
    else i++;
  }
}

function lexTla(text, out) {
  const n = text.length;
  let i = 0;
  while (i < n) {
    if (text[i] === "\\" && text[i + 1] === "*") {
      const end = lineEnd(text, i);
      out.comments.push({ start: i, end });
      i = end;
    } else if (text[i] === "(" && text[i + 1] === "*") {
      let depth = 1;
      let j = i + 2;
      while (j < n && depth > 0) {
        if (text[j] === "(" && text[j + 1] === "*") {
          depth++;
          j += 2;
        } else if (text[j] === "*" && text[j + 1] === ")") {
          depth--;
          j += 2;
        } else j++;
      }
      out.comments.push({ start: i, end: j });
      i = j;
    } else if (text[i] === '"') i = skipQuoted(text, i, '"');
    else i++;
  }
}

function lexHashLines(text, out, ctx) {
  const semicolon = /\.(service|ini|conf)$|\.editorconfig$/.test(ctx.path);
  for (const start of lineStarts(text)) {
    const end = lineEnd(text, start);
    const line = text.slice(start, end);
    const body = line.trimStart();
    if (body.startsWith("#") || (semicolon && body.startsWith(";"))) {
      out.comments.push({ start: start + line.length - body.length, end });
    }
  }
}

const LEXERS = {
  rust: lexRust,
  js: lexJs,
  shell: lexShell,
  swift: lexSwift,
  yaml: lexYaml,
  toml: lexToml,
  dockerfile: lexDockerfile,
  css: lexCss,
  markup: lexMarkup,
  sql: lexSql,
  jsonc: lexJsonc,
  tla: lexTla,
  "hash-lines": lexHashLines,
};
