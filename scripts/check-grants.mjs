import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, lstatSync, readFileSync, writeFileSync, appendFileSync } from "node:fs";
import { join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { languageOf, lineEnd, lineStarts, scanFiles } from "./grants/lex.mjs";
import { fillEmptyPythonBodies, stripComments } from "./grants/strip.mjs";

export const LEDGER = "grants.json";
export const JUDGE = ["scripts/check-grants.mjs", "scripts/grants", ".github/workflows"];
export const PROSE = /(\.(md|mdx|markdown|rst|adoc|asciidoc|txt|text|org|html?)|(^|\/)(LICENSE|COPYING|NOTICE)(-[^/]*)?)$/i;
export const DATA = [
  {
    path: /(^|\/)Cargo\.lock$/,
    reason: "Cargo writes it from Cargo.toml and nobody edits it by hand; a change to it is reviewed through the manifest that caused it",
  },
  {
    path: /(^|\/)patches\/[^/]+\.patch$/,
    reason: "pnpm applies it to a published package; its lines are that package's source in that package's language, not this repository's code",
  },
];
const KINDS = new Set(["comment", "suppression", "document"]);
const QUOTED = /["\\\u0000-\u001f\u007f]/;
const GRANT_COMMAND = "node " + relative(process.cwd(), fileURLToPath(import.meta.url)) + " grant";
const HASH_COMMAND = "node " + relative(process.cwd(), fileURLToPath(import.meta.url)) + " hash";
const READERS = relative(process.cwd(), fileURLToPath(new URL("./grants/lex.mjs", import.meta.url)));
const DATA_LIST = relative(process.cwd(), fileURLToPath(import.meta.url));

const DIRECTIVES = [
  (unit) => unit.line === 1 && unit.raw.startsWith("#!"),
  (unit) => unit.lang === "js" && /^\/\/\/\s*<reference\s[^>]*\/>$/.test(unit.raw),
  (unit) => unit.lang === "js" && /^@(vitest-environment|jest-environment|jsxImportSource)\s+\S+$|^@ts-check$/.test(unit.body),
  (unit) => /^SPDX-License-Identifier:\s*\S+$/.test(unit.body),
  (unit) => unit.lang === "python" && unit.line <= 2 && /coding[:=]\s*[-\w.]+/.test(unit.body),
  (unit) => unit.lang === "shell" && /^shellcheck\s+(shell|source|source-path|external-sources)=\S+$/.test(unit.body),
  (unit) => unit.lang === "yaml" && /^yaml-language-server:\s*\$schema=\S+$/.test(unit.body),
  (unit) => unit.lang === "swift" && unit.line === 1 && /^swift-tools-version:/.test(unit.body),
  (unit) =>
    unit.lang === "yaml" &&
    /^#\s*v\d+(\.\d+)*$/.test(unit.raw) &&
    /\buses:\s*[\w.-]+\/[\w./-]+@[0-9a-f]{40}\s*$/.test(unit.before.slice(unit.before.lastIndexOf("\n") + 1)),
  (unit) =>
    unit.lang === "dockerfile" &&
    /^#\s*(syntax|escape|check)\s*=\s*\S+$/.test(unit.raw) &&
    unit.before.split("\n").every((line) => line === "" || /^#\s*(syntax|escape|check)\s*=/.test(line)),
];

const COMMENT_SUPPRESSIONS = [
  [/@ts-(ignore|expect-error|nocheck)\b/, (m) => "@ts-" + m[1]],
  [/\bbiome-ignore/, () => "biome-ignore"],
  [/\b(eslint|oxlint)-disable/, (m) => m[1] + "-disable"],
  [/\bshellcheck\s+disable=/, () => "shellcheck disable"],
  [/(^|\s)noqa\b/i, () => "noqa"],
  [/\btype:\s*ignore\b/, () => "type: ignore"],
  [/\bpyright:\s*ignore\b/, () => "pyright: ignore"],
  [/\bpylint:\s*disable\b/, () => "pylint: disable"],
  [/\byamllint\s+disable/, () => "yamllint disable"],
  [/\bhadolint\s+ignore=/, () => "hadolint ignore"],
  [/\bswiftlint:disable\b/, () => "swiftlint:disable"],
  [/\bstylelint-disable/, () => "stylelint-disable"],
  [/\bprettier-ignore\b/, () => "prettier-ignore"],
  [/\b(istanbul|c8|v8)\s+ignore\b/, (m) => m[1] + " ignore"],
];

const FIXES = {
  comment:
    "A comment. If it states a constraint, encode it as a type, an assertion or a test that fails when it is broken. If it explains why, put that in the commit message. If it excuses a workaround, fix the cause.",
  suppression:
    "A suppression. Fix what the check found and delete the suppression. A skipped or focused test: make it pass in the gate, or delete it. A sleep in a test or script: wait on the event if the system offers one; otherwise poll the real condition in a loop whose bound the lint can see, a counted for loop or one that reads the clock, and that re-checks the condition each attempt through its own condition or an exit under an if. A break, return or exit that every attempt reaches checks nothing.",
  unread:
    "A text file of a type the lint has no reader for, so it cannot see what the file says. Add a reader for the type to languageOf in " +
    READERS +
    ", or, if the file is data that carries no words of ours, add it to DATA in " +
    DATA_LIST +
    " with the reason.",
  name: "A file name with a double quote, a backslash or a control character, which git quotes, so the lint cannot match the file to its lines. Rename the file.",
  document:
    "A new prose file. Say it where it is enforced or read instead: a rule becomes a check or a test, history goes in the commit message, and design goes in the section of an existing document that docs/ROUTING.md names.",
};

export function hashText(text) {
  return createHash("sha256").update(text).digest("hex");
}

function normalize(text) {
  return text
    .split("\n")
    .map((line) => line.trim())
    .join("\n");
}

function bodyOf(raw) {
  return raw
    .split("\n")
    .map((line) =>
      line
        .trim()
        .replace(/^(\/\/+!?|\/\*+!?|#+!?|<!--|--|;|\\\*|\(\*)\s?/, "")
        .replace(/\s*(\*\/|-->|\*\))$/, "")
        .replace(/^\*+\s?/, "")
        .trim(),
    )
    .join("\n")
    .trim();
}

function suppressionIn(body) {
  for (const [pattern, name] of COMMENT_SUPPRESSIONS) {
    const match = pattern.exec(body);
    if (match) return name(match);
  }
  return null;
}

export function unitsOf(path, text, scan) {
  const starts = lineStarts(text);
  const lineOf = (offset) => {
    let lo = 0;
    let hi = starts.length - 1;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if (starts[mid] <= offset) lo = mid;
      else hi = mid - 1;
    }
    return lo + 1;
  };
  const raws = [...scan.comments]
    .sort((a, b) => a.start - b.start)
    .map((span) => {
      const line = lineOf(span.start);
      const raw = text.slice(span.start, span.end).replace(/\s+$/, "");
      const unit = {
        path,
        lang: scan.lang,
        start: span.start,
        end: span.end,
        line,
        endLine: lineOf(Math.max(span.start, span.end - 1)),
        own: text.slice(starts[line - 1], span.start).trim() === "",
        raw,
        body: bodyOf(raw),
        before: text.slice(0, span.start),
      };
      const what = suppressionIn(unit.body);
      unit.kind = what ? "suppression" : DIRECTIVES.some((rule) => rule(unit)) ? "allowed" : "comment";
      unit.what = what ?? (unit.raw.startsWith("///") || unit.raw.startsWith("//!") || unit.raw.startsWith("/**") ? "doc comment" : "comment");
      return unit;
    });
  const units = [];
  for (const unit of raws) {
    const previous = units[units.length - 1];
    const joins =
      previous &&
      previous.kind === "comment" &&
      unit.kind === "comment" &&
      previous.own &&
      unit.own &&
      unit.line === unit.endLine &&
      previous.line === previous.endLine - (previous.endLine - previous.line) &&
      unit.line === previous.endLine + 1 &&
      previous.raw.split("\n").every((line) => !line.trim().startsWith("/*")) &&
      !unit.raw.startsWith("/*") &&
      previous.raw.trim().slice(0, 2) === unit.raw.trim().slice(0, 2);
    if (joins) {
      previous.end = unit.end;
      previous.endLine = unit.endLine;
      previous.raw = previous.raw + "\n" + unit.raw;
      previous.body = bodyOf(previous.raw);
    } else units.push(unit);
  }
  for (const unit of units) {
    unit.text = normalize(unit.raw);
    delete unit.before;
  }
  for (const mark of scan.marks) {
    const line = lineOf(mark.start);
    const endLine = lineOf(Math.max(mark.start, mark.end - 1));
    const source = text.slice(starts[line - 1], lineEnd(text, starts[endLine - 1]));
    units.push({ path, lang: scan.lang, line, endLine, kind: "suppression", what: mark.what, raw: source.trim(), text: normalize(source) });
  }
  for (const unit of units) unit.sha256 = hashText(unit.text);
  return units.sort((a, b) => a.line - b.line);
}

function git(repo, args, input) {
  return execFileSync("git", ["-c", "core.quotePath=false", ...args], { cwd: repo, encoding: "utf8", maxBuffer: 1 << 30, input, stdio: ["pipe", "pipe", "pipe"] });
}

function gitOk(repo, args) {
  try {
    return git(repo, args).trim();
  } catch {
    return null;
  }
}

export function approvalsFrom(env = process.env) {
  return (env.GRANTS_APPROVALS ?? "").split(/[\s,]+/).filter(Boolean);
}

export function resolveBase(repo, env = process.env) {
  if (env.GRANTS_BASE) return { rev: git(repo, ["rev-parse", env.GRANTS_BASE]).trim(), label: env.GRANTS_BASE };
  const branch = gitOk(repo, ["rev-parse", "--abbrev-ref", "HEAD"]);
  const onMain = !env.GITHUB_BASE_REF && (env.GITHUB_REF === "refs/heads/main" || branch === "main");
  if (onMain) {
    const parent = gitOk(repo, ["rev-parse", "HEAD^"]);
    if (!parent) throw new Error("HEAD has no parent commit to compare with; fetch more history (git fetch --deepen=1)");
    return { rev: parent, label: "the previous commit on main" };
  }
  const target = env.GITHUB_BASE_REF || "main";
  for (const ref of ["origin/" + target, target]) {
    const base = gitOk(repo, ["merge-base", "HEAD", ref]);
    if (base) return { rev: base, label: "the merge base with " + ref };
  }
  throw new Error("no merge base with origin/" + target + "; fetch it (git fetch origin " + target + ") or set GRANTS_BASE");
}

export function changedLines(repo, base, head) {
  const args = ["diff", "-M", "-w", "--unified=0", "--no-color", "--no-ext-diff", base];
  if (head) args.push(head);
  const changes = new Map();
  let current = null;
  for (const line of git(repo, args).split("\n")) {
    if (line.startsWith("diff --git ")) current = null;
    else if (line.startsWith("+++ ")) {
      current = line === "+++ /dev/null" || line.startsWith('+++ "') ? null : line.slice(6);
      if (current && !changes.has(current)) changes.set(current, new Set());
    } else if (line.startsWith("@@") && current) {
      const hunk = /\+(\d+)(?:,(\d+))?/.exec(line);
      const from = Number(hunk[1]);
      const count = hunk[2] === undefined ? 1 : Number(hunk[2]);
      for (let n = from; n < from + count; n++) changes.get(current).add(n);
    }
  }
  if (!head) {
    for (const path of git(repo, ["ls-files", "-z", "--others", "--exclude-standard"]).split("\u0000").filter(Boolean)) {
      changes.set(path, "all");
    }
  }
  return changes;
}

function readBlobs(repo, rev, paths) {
  const files = new Map();
  if (paths.length === 0) return files;
  if (!rev) {
    for (const path of paths) {
      const full = join(repo, path);
      if (existsSync(full)) files.set(path, readFileSync(full));
    }
    return files;
  }
  const out = execFileSync("git", ["cat-file", "--batch"], {
    cwd: repo,
    input: paths.map((path) => rev + ":" + path).join("\n") + "\n",
    maxBuffer: 1 << 30,
  });
  let at = 0;
  for (const path of paths) {
    const newline = out.indexOf(10, at);
    const header = out.subarray(at, newline).toString();
    at = newline + 1;
    if (header.endsWith("missing")) continue;
    const size = Number(header.split(" ")[2]);
    files.set(path, out.subarray(at, at + size));
    at += size + 1;
  }
  return files;
}

function asText(buffer) {
  return buffer.includes(0) ? null : buffer.toString("utf8");
}

export function classOf(path, text) {
  if (text === null) return "binary";
  if (languageOf(path, text)) return "code";
  if (PROSE.test(path)) return "prose";
  if (DATA.some((entry) => entry.path.test(path))) return "data";
  if (text.trim() === "") return "empty";
  return null;
}

function linksAmong(repo, rev, paths) {
  if (!rev) return new Set(paths.filter((path) => existsSync(join(repo, path)) && lstatSync(join(repo, path)).isSymbolicLink()));
  if (paths.length === 0) return new Set();
  return new Set(
    git(repo, ["ls-tree", "-r", "-z", rev, "--", ...paths])
      .split("\u0000")
      .filter((entry) => entry.startsWith("120000 "))
      .map((entry) => entry.slice(entry.indexOf("\t") + 1)),
  );
}

function touchedPaths(repo, base, head, changes) {
  const args = ["diff", "-z", "-M", "--name-only", "--diff-filter=ACMRT", "--no-ext-diff", base];
  if (head) args.push(head);
  return new Set([...changes.keys(), ...git(repo, args).split("\u0000").filter(Boolean)]);
}

export function upstreamOf(ledger) {
  const entries = ledger?.upstream ?? [];
  return (path) =>
    entries.some((entry) => path.startsWith(entry.path) && !(entry.except ?? []).some((own) => path === entry.path + own));
}

function readLedger(repo, rev) {
  if (!rev) {
    const full = join(repo, LEDGER);
    return existsSync(full) ? readFileSync(full, "utf8") : null;
  }
  try {
    return git(repo, ["show", rev + ":" + LEDGER]);
  } catch {
    return null;
  }
}

export function canonical(value) {
  if (Array.isArray(value)) return "[" + value.map(canonical).join(",") + "]";
  if (value === null || typeof value !== "object") return JSON.stringify(value);
  return "{" + Object.keys(value).sort().map((key) => JSON.stringify(key) + ":" + canonical(value[key])).join(",") + "}";
}

function contentOf(ledgerText) {
  try {
    return ledgerText === null ? null : canonical(JSON.parse(ledgerText));
  } catch {
    return null;
  }
}

export function approvalFor(ledger) {
  return "grants-approved-" + hashText(canonical(ledger)).slice(0, 12);
}

function parseLedger(text, problems) {
  if (text === null) return null;
  try {
    return JSON.parse(text);
  } catch (error) {
    problems.push({ path: LEDGER, message: "is not valid JSON: " + error.message });
    return null;
  }
}

function listFiles(repo, rev) {
  if (rev) return git(repo, ["ls-tree", "-r", "-z", "--name-only", rev]).split("\u0000").filter(Boolean);
  return git(repo, ["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
    .split("\u0000")
    .filter((path) => path && existsSync(join(repo, path)));
}

function validateLedger(repo, ledger, head, problems) {
  const existing = ledger.existing ?? {};
  const files = new Set(listFiles(repo, head));
  if (gitOk(repo, ["cat-file", "-e", existing.commit + "^{commit}"]) === null) {
    problems.push({ path: LEDGER, message: "names existing.commit " + existing.commit + ", which is not in this clone; fetch full history (git fetch --unshallow)" });
    return { files, renamedFrom: new Map() };
  }
  const atCommit = new Set(git(repo, ["ls-tree", "-r", "--name-only", existing.commit]).split("\n"));
  const renameArgs = ["diff", "-M", "--name-status", "--diff-filter=R", existing.commit];
  if (head) renameArgs.push(head);
  const renamedFrom = new Map();
  const renamedAway = new Set();
  for (const line of git(repo, renameArgs).split("\n").filter(Boolean)) {
    const [, from, to] = line.split("\t");
    renamedFrom.set(to, from);
    renamedAway.add(from);
  }
  for (const path of existing.files ?? []) {
    if (!atCommit.has(path)) {
      problems.push({ path: LEDGER, message: path + " is listed as existing but was not a file at " + existing.commit.slice(0, 12) + "; a new document needs the owner's grant" });
    } else if (!files.has(path) && !renamedAway.has(path)) {
      problems.push({ path: LEDGER, message: path + " is listed as existing but is gone; remove it from " + LEDGER });
    }
  }
  for (const [index, grant] of (ledger.grants ?? []).entries()) {
    const where = "grant " + (index + 1) + " (" + grant.kind + " in " + grant.path + ")";
    if (!KINDS.has(grant.kind)) problems.push({ path: LEDGER, message: where + " has an unknown kind; use comment, suppression or document" });
    if (typeof grant.words !== "string" || grant.words.trim() === "") problems.push({ path: LEDGER, message: where + " has no words from the owner" });
    if (!/^\d{4}-\d{2}-\d{2}$/.test(grant.date ?? "")) problems.push({ path: LEDGER, message: where + " needs a date as YYYY-MM-DD" });
    if (grant.kind !== "document" && !/^[0-9a-f]{64}$/.test(grant.sha256 ?? "")) {
      problems.push({ path: LEDGER, message: where + " needs the sha256 of the text it grants" });
    }
  }
  return { files, renamedFrom };
}

function shippedMigrations(repo, ledger, head, problems) {
  const listed = ledger?.migrations;
  if (!listed) return new Set();
  if (gitOk(repo, ["cat-file", "-e", listed.commit + "^{commit}"]) === null) {
    problems.push({ path: LEDGER, message: "names migrations.commit " + listed.commit + ", which is not in this clone; fetch full history (git fetch --unshallow)" });
    return new Set();
  }
  const shipped = new Set();
  for (const path of listed.files ?? []) {
    const then = gitOk(repo, ["rev-parse", listed.commit + ":" + path]);
    const now = head ? gitOk(repo, ["rev-parse", head + ":" + path]) : existsSync(join(repo, path)) ? gitOk(repo, ["hash-object", "--", path]) : null;
    if (then === null) {
      problems.push({ path: LEDGER, message: path + " is listed as a shipped migration but was not a file at " + listed.commit.slice(0, 12) + "; a new migration gets no exemption" });
    } else if (now === null) {
      problems.push({ path: LEDGER, message: path + " is listed as a shipped migration but is gone; remove it from " + LEDGER });
    } else if (now !== then) {
      problems.push({ path: LEDGER, message: path + " is listed as a shipped migration but differs from " + listed.commit.slice(0, 12) + "; a shipped migration is never edited, so add a new one" });
    } else shipped.add(path);
  }
  return shipped;
}

function judgeChange(repo, base, head, approvals) {
  const edited = git(repo, ["diff", "--name-only", "--no-renames", base, head, "--", ...JUDGE]).split("\n").filter(Boolean);
  if (edited.length === 0) return [];
  const approval = "judge-approved-" + hashText(git(repo, ["ls-tree", "-r", head, "--", ...JUDGE])).slice(0, 12);
  if (approvals.includes(approval)) return [];
  return [
    {
      path: edited.join(", "),
      message:
        "changes the lint that judges every pull request, and only the owner approves a change to the judge. Stop here: report what the change does to the judge to whoever directs the work, and wait. The author never approves their own change to the judge. The approval is the pull request label " +
        approval +
        ", made from the SHA-256 of every file of the judge at this head (" +
        JUDGE.join(", ") +
        "), so any later change to them needs a new approval.",
    },
  ];
}

export function check(repo, { base, head = null, ledgerOverride, proseByDiff = false, approvals = [], ledgerGate = true, judge = false }) {
  if (judge && head === null) throw new Error("the judge reads a committed head as data; name it with --head");
  const problems = [];
  const findings = [];
  const ledgerText = ledgerOverride === undefined ? readLedger(repo, head) : ledgerOverride;
  const ledger = parseLedger(ledgerText, problems);
  const upstream = upstreamOf(ledger);
  const changes = changedLines(repo, base, head);
  const grants = ledger?.grants ?? [];
  const grantFiles = grants.filter((grant) => grant.kind !== "document").map((grant) => grant.path);
  const shipped = shippedMigrations(repo, ledger, head, problems);
  const judgedWhole = (path) => !PROSE.test(path) && !shipped.has(path);
  const named = (path) => !upstream(path) && !QUOTED.test(path);
  const files = listFiles(repo, head).filter((path) => !upstream(path));
  for (const path of files.filter((path) => QUOTED.test(path))) findings.push({ path, line: null, kind: "name", what: "a file name git quotes" });
  const candidates = [...touchedPaths(repo, base, head, changes)].filter(named);
  const tree = files.filter((path) => named(path) && judgedWhole(path));
  const links = linksAmong(repo, head, [...new Set([...candidates, ...tree])]);
  const touched = candidates.filter((path) => !links.has(path));
  const wanted = [...new Set([...touched, ...grantFiles, ...tree])].filter((path) => !upstream(path) && !links.has(path));
  const blobs = readBlobs(repo, head, wanted);
  for (const path of touched) {
    const blob = blobs.get(path);
    if (blob && classOf(path, asText(blob)) === null) findings.push({ path, line: null, kind: "unread", what: "no reader for this type" });
  }
  const texts = [];
  for (const [path, blob] of blobs) {
    const text = asText(blob);
    if (text !== null) texts.push({ path, text });
  }
  const scans = scanFiles(texts);
  const unitsByPath = new Map();
  for (const { path, text } of texts) {
    const scan = scans.get(path);
    if (!scan) continue;
    if (scan.error) {
      if (changes.has(path)) problems.push({ path, message: "could not be read as " + scan.lang + ": " + scan.error });
      continue;
    }
    unitsByPath.set(path, unitsOf(path, text, scan));
  }
  const key = (kind, path, sha256) => kind + "\u0000" + path + "\u0000" + sha256;
  const available = new Map();
  for (const grant of grants) {
    if (grant.kind === "document") continue;
    const k = key(grant.kind, grant.path, grant.sha256);
    available.set(k, (available.get(k) ?? 0) + 1);
  }
  const instances = new Map();
  for (const units of unitsByPath.values()) {
    for (const unit of units) {
      if (unit.kind === "allowed") continue;
      const k = key(unit.kind, unit.path, unit.sha256);
      instances.set(k, (instances.get(k) ?? 0) + 1);
    }
  }
  for (const [path, units] of unitsByPath) {
    const lines = changes.get(path) ?? new Set();
    for (const unit of units) {
      if (unit.kind === "allowed") continue;
      let touched = lines === "all" || (unit.kind === "comment" && judgedWhole(path));
      for (let n = unit.line; !touched && n <= unit.endLine; n++) touched = lines.has(n);
      if (!touched) continue;
      const k = key(unit.kind, unit.path, unit.sha256);
      const left = available.get(k) ?? 0;
      if (left > 0) {
        available.set(k, left - 1);
        continue;
      }
      findings.push(unit);
    }
  }
  if (ledger) {
    const counted = new Map();
    for (const grant of grants) {
      if (grant.kind === "document") continue;
      const k = key(grant.kind, grant.path, grant.sha256);
      counted.set(k, (counted.get(k) ?? 0) + 1);
      if (counted.get(k) > (instances.get(k) ?? 0)) {
        problems.push({ path: LEDGER, message: "a " + grant.kind + " grant for " + grant.path + " (" + grant.sha256.slice(0, 12) + ") matches nothing; remove it" });
      }
    }
  }
  if (ledger && !proseByDiff) {
    const { files, renamedFrom } = validateLedger(repo, ledger, head, problems);
    const existing = new Set(ledger.existing?.files ?? []);
    const documents = new Set(grants.filter((grant) => grant.kind === "document").map((grant) => grant.path));
    for (const path of documents) {
      if (!files.has(path)) problems.push({ path: LEDGER, message: "a document grant for " + path + " matches no file; remove it" });
    }
    for (const path of files) {
      if (!PROSE.test(path) || upstream(path) || existing.has(path) || documents.has(path)) continue;
      if (existing.has(renamedFrom.get(path))) continue;
      findings.push({ path, line: null, kind: "document", what: "new document" });
    }
  } else {
    const args = ["diff", "-M", "--name-status", "--diff-filter=A", base];
    if (head) args.push(head);
    const added = git(repo, args)
      .split("\n")
      .filter(Boolean)
      .map((line) => line.split("\t")[1]);
    if (!head) added.push(...[...changes].filter(([, lines]) => lines === "all").map(([path]) => path));
    for (const path of new Set(added)) {
      if (PROSE.test(path) && !upstream(path)) findings.push({ path, line: null, kind: "document", what: "new document" });
    }
  }
  const baseText = readLedger(repo, base);
  let approval = null;
  const content = contentOf(ledgerText);
  if (ledgerGate && (ledgerText === null || content !== null) && content !== contentOf(baseText)) {
    approval = approvalFor(ledger);
    if (!approvals.includes(approval)) {
      problems.push({
        path: LEDGER,
        message:
          "differs from the base, and only the owner approves a ledger change. Stop here: report the exact text of each grant above, and what you tried instead, to whoever directs the work, and wait. The author never approves their own ledger change. The approval is the pull request label " +
          approval +
          ", made from the SHA-256 of what this " +
          LEDGER +
          " says, so any later change to what it says needs a new approval and a change to its formatting alone does not. " +
          HASH_COMMAND +
          " prints it.",
      });
    }
  }
  if (judge) problems.push(...judgeChange(repo, base, head, approvals));
  return { findings, problems, filesRead: texts.length, approval, ledgerChanges: ledger ? ledgerChanges(baseText, ledger, unitsByPath) : null };
}

function grantKey(grant) {
  return [grant.kind, grant.path, grant.sha256 ?? ""].join(" ");
}

function ledgerChanges(baseText, ledger, unitsByPath) {
  let before = {};
  try {
    before = baseText === null ? {} : JSON.parse(baseText);
  } catch {
    before = {};
  }
  const describe = (grant) => {
    const unit = (unitsByPath.get(grant.path) ?? []).find((u) => u.sha256 === grant.sha256 && u.kind === grant.kind);
    const where = unit ? grant.path + ":" + unit.line : grant.path;
    const text = unit ? "\n      " + unit.raw.split("\n").join("\n      ") : "";
    return grant.kind + " in " + where + ", " + grant.date + ', "' + grant.words + '"' + text;
  };
  const tally = (list) => {
    const map = new Map();
    for (const grant of list ?? []) map.set(grantKey(grant), [...(map.get(grantKey(grant)) ?? []), grant]);
    return map;
  };
  const was = tally(before.grants);
  const now = tally(ledger.grants);
  const added = [];
  const removed = [];
  const altered = [];
  for (const k of new Set([...was.keys(), ...now.keys()])) {
    const a = was.get(k) ?? [];
    const b = now.get(k) ?? [];
    for (let i = 0; i < Math.max(a.length, b.length); i++) {
      if (!a[i]) added.push(describe(b[i]));
      else if (!b[i]) removed.push(describe(a[i]));
      else if (a[i].words !== b[i].words || a[i].date !== b[i].date) altered.push(describe(a[i]) + "\n    now " + describe(b[i]));
    }
  }
  if (baseText === null) {
    return [
      "  + new ledger: " + (ledger.existing?.files ?? []).length + " prose files recorded as existing at " + ledger.existing?.commit,
      ...(ledger.upstream ?? []).map((entry) => "  + upstream " + JSON.stringify(entry)),
      ...added.map((item) => "  + added    " + item),
    ];
  }
  const setDiff = (x, y) => (x ?? []).filter((item) => !(y ?? []).includes(item));
  const upstreamText = (list) => (list ?? []).map((entry) => JSON.stringify(entry));
  const lines = [];
  for (const item of added) lines.push("  + added    " + item);
  for (const item of removed) lines.push("  - removed  " + item);
  for (const item of altered) lines.push("  ~ altered  " + item);
  for (const path of setDiff(ledger.existing?.files, before.existing?.files)) lines.push("  + existing " + path);
  for (const path of setDiff(before.existing?.files, ledger.existing?.files)) lines.push("  - existing " + path);
  if ((before.existing?.commit ?? null) !== (ledger.existing?.commit ?? null)) {
    lines.push("  ~ existing.commit " + (before.existing?.commit ?? "none") + " -> " + ledger.existing?.commit);
  }
  for (const path of setDiff(ledger.migrations?.files, before.migrations?.files)) lines.push("  + shipped migration " + path);
  for (const path of setDiff(before.migrations?.files, ledger.migrations?.files)) lines.push("  - shipped migration " + path);
  if ((before.migrations?.commit ?? null) !== (ledger.migrations?.commit ?? null)) {
    lines.push("  ~ migrations.commit " + (before.migrations?.commit ?? "none") + " -> " + (ledger.migrations?.commit ?? "none"));
  }
  for (const entry of setDiff(upstreamText(ledger.upstream), upstreamText(before.upstream))) lines.push("  + upstream " + entry);
  for (const entry of setDiff(upstreamText(before.upstream), upstreamText(ledger.upstream))) lines.push("  - upstream " + entry);
  return lines;
}

export function report(result, base) {
  const out = [];
  const { findings, problems } = result;
  if (result.ledgerChanges?.length) {
    out.push(
      "",
      "GRANTS CHANGED. This change edits " + LEDGER + ". Only the owner grants an exception; check each line below against the owner's own words before merging:",
      ...result.ledgerChanges,
      "",
    );
  }
  if (findings.length === 0 && problems.length === 0) {
    out.push("grants: nothing new since " + base.rev.slice(0, 12) + " (" + base.label + "), " + result.filesRead + " files read");
    return out.join("\n");
  }
  out.push("grants: " + (findings.length + problems.length) + " to fix since " + base.rev.slice(0, 12) + " (" + base.label + ")", "");
  for (const finding of findings) {
    const where = finding.kind === "name" ? JSON.stringify(finding.path) : finding.line ? finding.path + ":" + finding.line : finding.path;
    const label = finding.kind === "document" ? "new document" : finding.what;
    const first = finding.raw ? "  " + finding.raw.split("\n")[0].trim().slice(0, 100) : "";
    out.push(where + "  " + label + first);
  }
  for (const problem of problems) out.push(problem.path + "  " + problem.message);
  const kinds = new Set(findings.map((finding) => finding.kind));
  if (kinds.size > 0) out.push("");
  if (kinds.has("unread")) out.push(FIXES.unread);
  if (kinds.has("name")) out.push(FIXES.name);
  for (const kind of ["comment", "suppression", "document"]) {
    if (!kinds.has(kind)) continue;
    const target = kind === "document" ? "<file>" : "<file>:<line>";
    out.push(FIXES[kind] + " Only if none of that can work, ask the owner, then record their words: " + GRANT_COMMAND + " " + target + ' --words "<the owner\'s words>"');
  }
  return out.join("\n");
}

function annotate(result) {
  if (!process.env.GITHUB_ACTIONS) return;
  for (const finding of result.findings) {
    const line = finding.line ? ",line=" + finding.line : "";
    const message =
      finding.kind === "unread"
        ? "no reader for this file type; add one, or list the file as data with the reason"
        : (finding.kind === "document" ? "new document" : finding.what) + " needs the owner's grant";
    console.log("::error file=" + finding.path + line + "::" + message);
  }
  if (result.ledgerChanges?.length) {
    console.log("::warning file=" + LEDGER + ",title=Grants changed::" + result.ledgerChanges.length + " ledger changes; read them before merging");
    if (process.env.GITHUB_STEP_SUMMARY) {
      const approval = result.approval ? "The approval for this exact ledger is the pull request label `" + result.approval + "`.\n\n" : "";
      appendFileSync(process.env.GITHUB_STEP_SUMMARY, "## Grants changed\n\nOnly the owner grants. Read each before merging.\n\n" + approval + "~~~\n" + result.ledgerChanges.join("\n") + "\n~~~\n");
    }
  }
}

export function grant(repo, target, words, date) {
  const ledgerPath = join(repo, LEDGER);
  const ledger = JSON.parse(readFileSync(ledgerPath, "utf8"));
  const match = /^(.*?)(?::(\d+))?$/.exec(target);
  const path = relative(repo, resolve(repo, match[1]));
  let entry;
  if (!match[2]) {
    if (!PROSE.test(path)) throw new Error(path + " is not a prose file; name a line for a comment or a suppression: <file>:<line>");
    entry = { kind: "document", path, words, date };
  } else {
    const line = Number(match[2]);
    const text = readFileSync(join(repo, path), "utf8");
    const scan = scanFiles([{ path, text }]).get(path);
    if (!scan || scan.error) throw new Error("cannot read comments in " + path + (scan?.error ? ": " + scan.error : ""));
    const unit = unitsOf(path, text, scan).find((u) => u.kind !== "allowed" && u.line <= line && line <= u.endLine);
    if (!unit) throw new Error("no comment or suppression at " + path + ":" + line);
    entry = { kind: unit.kind, path, sha256: unit.sha256, words, date };
  }
  ledger.grants = [...(ledger.grants ?? []), entry];
  writeFileSync(ledgerPath, JSON.stringify(ledger, null, 2) + "\n");
  const biome = join(repo, "node_modules", ".bin", "biome");
  if (existsSync(biome)) execFileSync(biome, ["format", "--write", LEDGER], { cwd: repo, stdio: "ignore" });
  return entry;
}

export function sweep(repo, { only = [] } = {}) {
  const ledgerText = readLedger(repo, null);
  const ledger = ledgerText === null ? null : JSON.parse(ledgerText);
  const upstream = upstreamOf(ledger);
  const migrations = new Set(ledger?.migrations?.files ?? []);
  const wanted = (path) => only.length === 0 || only.some((prefix) => path === prefix || path.startsWith(prefix.replace(/\/?$/, "/")));
  const candidates = listFiles(repo, null).filter((path) => wanted(path) && !upstream(path) && !PROSE.test(path) && !migrations.has(path));
  const links = linksAmong(repo, null, candidates);
  const texts = [];
  for (const path of candidates) {
    if (links.has(path)) continue;
    const text = asText(readFileSync(join(repo, path)));
    if (text !== null && classOf(path, text) === "code") texts.push({ path, text });
  }
  const scans = scanFiles(texts);
  const available = new Map();
  for (const grant of ledger?.grants ?? []) {
    if (grant.kind !== "comment") continue;
    const k = grant.path + "\u0000" + grant.sha256;
    available.set(k, (available.get(k) ?? 0) + 1);
  }
  const removed = [];
  const unreadable = [];
  const changed = [];
  for (const { path, text } of texts) {
    const scan = scans.get(path);
    if (scan.error) {
      unreadable.push({ path, message: scan.error });
      continue;
    }
    const units = unitsOf(path, text, scan).filter((unit) => {
      if (unit.kind !== "comment") return false;
      const k = unit.path + "\u0000" + unit.sha256;
      const left = available.get(k) ?? 0;
      if (left > 0) available.set(k, left - 1);
      return left === 0;
    });
    if (units.length === 0) continue;
    const result = stripComments(text, units, { dialect: languageOf(path, text).dialect });
    writeFileSync(join(repo, path), scan.lang === "python" ? fillEmptyPythonBodies(result.text) : result.text);
    changed.push(path);
    removed.push(...result.removed.map((unit) => ({ path, lang: unit.lang, line: unit.line, endLine: unit.endLine, what: unit.what, raw: unit.raw })));
  }
  return { removed, unreadable, changed };
}

function formatChanged(repo, changed) {
  for (const path of changed.filter((file) => file.endsWith(".rs"))) {
    const full = join(repo, path);
    let dir = join(full, "..");
    while (!existsSync(join(dir, "Cargo.toml")) && dir !== repo) dir = join(dir, "..");
    const manifest = join(dir, "Cargo.toml");
    const edition = existsSync(manifest) ? /^edition\s*=\s*"(\d+)"/m.exec(readFileSync(manifest, "utf8"))?.[1] : undefined;
    const formatted = execFileSync("rustfmt", [...(edition ? ["--edition", edition] : []), "--emit", "stdout"], { cwd: join(full, ".."), input: readFileSync(full), maxBuffer: 1 << 28 });
    writeFileSync(full, formatted);
  }
  const biome = join(repo, "node_modules", ".bin", "biome");
  if (existsSync(biome) && changed.length > 0) {
    execFileSync(biome, ["format", "--write", "--no-errors-on-unmatched", ...changed], { cwd: repo, stdio: ["ignore", "ignore", "inherit"] });
  }
}

function parseArgs(argv) {
  const options = { positional: [] };
  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg.startsWith("--")) options[arg.slice(2)] = argv[++i];
    else options.positional.push(arg);
  }
  return options;
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  const repo = git(options.repo ?? process.cwd(), ["rev-parse", "--show-toplevel"]).trim();
  if (options.positional[0] === "grant") {
    if (!options.words) throw new Error('grant needs the owner\'s words: --words "..."');
    const date = options.date ?? new Date().toISOString().slice(0, 10);
    const entry = grant(repo, options.positional[1], options.words, date);
    console.log("recorded " + JSON.stringify(entry) + " in " + LEDGER + "; the gate prints it to whoever merges this change");
    return 0;
  }
  if (options.positional[0] === "hash") {
    console.log(approvalFor(JSON.parse(readLedger(repo, null) ?? "null")));
    return 0;
  }
  if (options.positional[0] === "strip") {
    const result = sweep(repo, { only: options.positional.slice(1) });
    if (options.record) {
      const escape = (text) => text.replace(/\\/g, "\\\\").replace(/\t/g, "\\t").replace(/\n/g, "\\n");
      const rows = result.removed.map((unit) => [unit.path, unit.line, unit.endLine, unit.lang, unit.what, escape(unit.raw)].join("\t"));
      writeFileSync(options.record, ["path\tline\tend_line\tlanguage\tkind\ttext", ...rows].join("\n") + "\n");
    }
    formatChanged(repo, result.changed);
    for (const problem of result.unreadable) console.error(problem.path + "  could not be read: " + problem.message);
    console.log("removed " + result.removed.length + " comments from " + result.changed.length + " files");
    return result.unreadable.length > 0 ? 1 : 0;
  }
  const base = options.base ? { rev: git(repo, ["rev-parse", options.base]).trim(), label: options.base } : resolveBase(repo);
  const pushToMain = process.env.GITHUB_EVENT_NAME === "push" && process.env.GITHUB_REF === "refs/heads/main";
  const judge = options.positional[0] === "judge";
  const head = options.head ?? (judge ? "HEAD" : null);
  const result = check(repo, { base: base.rev, head, approvals: approvalsFrom(), ledgerGate: !pushToMain, judge });
  annotate(result);
  console.log(report(result, base));
  return result.findings.length + result.problems.length > 0 ? 1 : 0;
}

if (process.argv[1] && resolve(process.argv[1]) === resolve(new URL(import.meta.url).pathname)) {
  try {
    process.exitCode = main();
  } catch (error) {
    console.error("grants: " + error.message);
    process.exitCode = 2;
  }
}
