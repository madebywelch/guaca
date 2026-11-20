import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import { frontendCopy, frontendDashes, RUST_COPY, retiredWords, rustCopy } from "./check-operator-copy.mjs";

test("finds literal copy without treating comments as copy", () => {
  const dash = "\u2014";
  const source = `// ${dash}\nconst a = "${dash}";\nconst b = <p>hello ${dash} there</p>;\nconst c = \`hello ${dash} ${"${name}"}\`;`;
  assert.deepEqual(frontendDashes(source, "sample.tsx"), [2, 3, 4]);
});

test("fails the gate for a new frontend source file", () => {
  const root = mkdtempSync(join(tmpdir(), "operator-copy-"));
  try {
    mkdirSync(join(root, "src", "components"), { recursive: true });
    mkdirSync(join(root, "src-tauri", "src"), { recursive: true });
    for (const file of ["account.rs", "oauth.rs", "commands.rs"]) {
      writeFileSync(join(root, "src-tauri", "src", file), "");
    }
    for (const file of RUST_COPY) {
      mkdirSync(join(root, file, ".."), { recursive: true });
      writeFileSync(join(root, file), "");
    }
    const guard = resolve("scripts/check-operator-copy.mjs");
    execFileSync(process.execPath, [guard], { cwd: root });
    writeFileSync(join(root, "src", "components", "New.tsx"), `export const copy = "bad \u2014 copy";`);
    const result = spawnSync(process.execPath, [guard], { cwd: root, encoding: "utf8" });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /src\/components\/New\.tsx:1/);
    writeFileSync(join(root, "src", "components", "New.tsx"), "");
    writeFileSync(join(root, "src-tauri", "src", "notice.rs"), `const NOTICE: &str = "bad \u2014 notice";`);
    const rust = spawnSync(process.execPath, [guard], { cwd: root, encoding: "utf8" });
    assert.equal(rust.status, 1);
    assert.match(rust.stderr, /src-tauri\/src\/notice\.rs:1/);
    writeFileSync(join(root, "src-tauri", "src", "notice.rs"), "");
    writeFileSync(join(root, "src", "new.css"), `.notice::before { content: "\u2014"; }`);
    const css = spawnSync(process.execPath, [guard], { cwd: root, encoding: "utf8" });
    assert.equal(css.status, 1);
    assert.match(css.stderr, /src\/new\.css:1/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

const RETIRED_COPY = [
  "1 item needs your attention.",
  "2 items need your attention.",
  "1 turn is waiting on you",
  "3 turns waiting on you",
  "2 decisions need a review. Open For you to answer or follow up.",
  "1 decision set aside in For you",
  "A permission request, a question, an agent that is stuck, and reminders about decisions in For you.",
  "Decision reminders at 9 AM and 4 PM",
  "Following through 2",
  "Team activity",
  "Open For You",
  "Open channel",
  "Open a channel",
  "Whose channel",
  "Opens the channel with Pip",
  "This request is no longer live.",
  "Cleared Pip's request for help.",
  "Select the next or the previous request",
  "Could not refresh decisions: offline",
  "Decision views",
  "2 pending decisions",
  "View channel",
  "Its turn is parked until you answer, and gives up after ten minutes.",
  "2 waiting on you",
  "For you, 1 waiting",
  "Allow the request the selected row belongs to",
  "3 decisions",
  "1 pending request",
  "Approve the request",
];

const CURRENT_COPY = [
  "Executive Assistant is waiting on you.",
  "Pip, Ada and 2 others are waiting on you",
  "For you, 1 question",
  "2 questions and 1 result in For you",
  "Reminders about open questions at 9 AM and 4 PM",
  "1 in progress after your answer.",
  "When each agent last worked",
  "Open conversation",
  "Answered in For you",
  "Research a decision",
  "Giving it one is a decision only you can make.",
  "The request did not complete.",
  "Testing makes a small model request that may be billed.",
  "a request for an access token when an agent uses a connector",
  "Review the pull request",
  "Review the pull request in For you",
  "Pip made a request for you",
  "Reminders turn on at 9 AM",
  "2 questions waiting on you",
  "For you, 1 question",
];

test("retires every word the waiting request was once called, and only those", () => {
  for (const text of RETIRED_COPY) assert.notDeepEqual(retiredWords(text), [], text);
  for (const text of CURRENT_COPY) assert.deepEqual(retiredWords(text), [], text);
});

test("reads a sentence split across a template, its branches and JSX as one piece of copy", () => {
  const source = [
    "const a = `${n} ${n === 1 ? \"item needs\" : \"items need\"} your attention.`;",
    "const b = <p>{n} {n === 1 ? \"turn\" : \"turns\"} waiting on you</p>;",
    "import { x } from \"./decisions\";",
    "const d = n + \" \" + w + (n === 1 ? \" item needs\" : \" items need\") + \" your attention\";",
    "const e = <p className=\"decision__answer\" title={\"Answered\"}>Waiting on you</p>;",
  ].join("\n");
  const copy = frontendCopy(source, "sample.tsx");
  assert.deepEqual(copy.filter((one) => retiredWords(one.text).length > 0).map((one) => one.line), [1, 2, 4]);
});

test("reads Rust string literals and skips comment lines", () => {
  const source = '// 1 item needs your attention\nlet t = format!("{n} items need your attention in For you");';
  assert.deepEqual(rustCopy(source).map((one) => one.line), [2]);
  assert.notDeepEqual(retiredWords(rustCopy(source)[0].text), []);
});

test("fails the gate when a retired word comes back in a component or the menu bar", () => {
  const root = mkdtempSync(join(tmpdir(), "operator-words-"));
  try {
    mkdirSync(join(root, "src", "components"), { recursive: true });
    for (const file of RUST_COPY) {
      mkdirSync(join(root, file, ".."), { recursive: true });
      writeFileSync(join(root, file), "");
    }
    const guard = resolve("scripts/check-operator-copy.mjs");
    execFileSync(process.execPath, [guard], { cwd: root });
    writeFileSync(join(root, "src", "components", "New.tsx"), "export const T = () => <h3>1 turn is waiting on you</h3>;");
    const web = spawnSync(process.execPath, [guard], { cwd: root, encoding: "utf8" });
    assert.equal(web.status, 1);
    assert.match(web.stderr, /src\/components\/New\.tsx:1/);
    writeFileSync(join(root, "src", "components", "New.tsx"), "");
    writeFileSync(join(root, RUST_COPY[0]), 'let s = format!("{w} items need your attention in For you");');
    const rust = spawnSync(process.execPath, [guard], { cwd: root, encoding: "utf8" });
    assert.equal(rust.status, 1);
    assert.match(rust.stderr, /src-tauri\/src\/menubar\.rs:1/);
    rmSync(join(root, RUST_COPY[0]));
    const gone = spawnSync(process.execPath, [guard], { cwd: root, encoding: "utf8" });
    assert.equal(gone.status, 1);
    assert.match(gone.stderr, /listed in RUST_COPY but missing/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
