import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import { check, hashText, PROSE, report, sweep, unitsOf } from "./check-grants.mjs";
import { scanFiles } from "./grants/lex.mjs";

const CLI = join(dirname(fileURLToPath(import.meta.url)), "check-grants.mjs");
const TICK = String.fromCharCode(96);
const INTERPOLATE = "$" + "{";

function found(path, text) {
  const scan = scanFiles([{ path, text }]).get(path);
  assert.ok(scan, "no lexer for " + path);
  assert.equal(scan.error, null);
  return unitsOf(path, text, scan).filter((unit) => unit.kind !== "allowed");
}

const LANGUAGES = [
  {
    path: "src/lib.rs",
    fail: [
      ["fn a() {} // trailing", "comment"],
      ["/// what a does\nfn a() {}", "doc comment"],
      ["//! the crate\nfn a() {}", "doc comment"],
      ["/* outer /* inner */ still outer */\nfn a() {}", "comment"],
      ["#[allow(dead_code)]\nfn a() {}", "#[allow]"],
      ["#![allow(clippy::all)]", "#[allow]"],
      ["#[expect(unused)]\nfn a() {}", "#[expect]"],
      ["#[cfg_attr(test, allow(dead_code))]\nfn a() {}", "#[cfg_attr(.., allow)]"],
      ['#[test]\n#[ignore = "slow"]\nfn t() {}', "#[ignore]"],
      ["#[rustfmt::skip]\nfn a() {}", "#[rustfmt::skip]"],
      ["#[cfg(test)]\nmod tests {\n    fn t() { std::thread::sleep(D); }\n}", "sleep"],
      ["// SAFETY: not followed by unsafe\nfn a() {}", "comment"],
      ["fn a(p: *const u8) -> u8 {\n    // SAFETY: the caller hands a valid pointer\n    unsafe { *p }\n}", "comment"],
      ["#[test]\nfn t() { loop { if ready() { break; } std::thread::sleep(D); } }", "sleep"],
      ["#[test]\nfn t() { for _ in 0..3 { std::thread::sleep(D); } }", "sleep"],
      ["#[test]\nfn t() { for _ in 0..1 { std::thread::sleep(D); break; } }", "sleep"],
      ["#[test]\nfn t() { for _ in 0..1 { std::thread::sleep(D); return; } }", "sleep"],
      ["#[test]\nfn t() { for _ in 0..1 { std::thread::sleep(D); { break; } } }", "sleep"],
      ["#[test]\nfn t() { loop { let _ = Instant::now(); std::thread::sleep(D); break; } }", "sleep"],
      ["#[test]\nfn t() { for _ in 0..1 { std::thread::sleep(D); if true { break; } } }", "sleep"],
    ],
    pass: [
      'fn a() { let u = "http://x//y /* no */"; }',
      'fn a() { let r = r#"// no "quote" /* no */"#; let b = br"//"; }',
      "fn a<'a>(s: &'a str) -> char { let c = '\"'; let d = '/'; '#' }",
      '#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]\nfn a() {}',
      "fn a() { std::thread::sleep(D); }",
      "#[cfg(test)]\nmod tests {\n    fn t() { for _ in 0..50 { if ready() { break; } std::thread::sleep(D); } }\n}",
      "#[test]\nfn t() {\n    while !check() {\n        assert!(Instant::now() < deadline);\n        std::thread::sleep(D);\n    }\n}",
      "impl Iterator for X { fn next(&mut self) -> Option<u8> { None } }\nfn main() {}",
      "#[test]\nfn t() { for _ in 0..50 { match probe() { Some(v) => return v, None => {} } std::thread::sleep(D); } }",
    ],
  },
  {
    path: "src/lib/a.ts",
    fail: [
      ["const a = 1; // trailing", "comment"],
      ["/** what a is */\nexport const a = 1;", "doc comment"],
      ["/* block */ const a = 1;", "comment"],
      ["// @ts-expect-error\nconst a: string = 1;", "@ts-expect-error"],
      ["// @ts-ignore\nconst a: string = 1;", "@ts-ignore"],
      ["// biome-ignore lint/style/x: reason\nconst a = 1;", "biome-ignore"],
      ["/* eslint-disable */\nconst a = 1;", "eslint-disable"],
    ],
    pass: [
      "const u = \"http://x//y\"; const s = '/* no */';",
      "const re = /\\/\\/ no/g; const d = 4 / 2 / 1;",
      "const t = " + TICK + "// no " + INTERPOLATE + "1 + 1} /* no */" + TICK + ";",
      '/// <reference types="vite/client" />\nexport {};',
    ],
  },
  {
    path: "src/lib/a.test.tsx",
    fail: [
      ['it.skip("x", () => {});', ".skip"],
      ['describe.only("x", () => {});', ".only"],
      ['test.todo("x");', ".todo"],
      ['it.skipIf(process.env.CI)("x", () => {});', ".skipIf"],
      ['test("x", { skip: true }, () => {});', "{ skip }"],
      ['it("x", async () => { await new Promise((resolve) => setTimeout(resolve, 10)); });', "sleep"],
      ['it("x", async () => { await new Promise((r) => setTimeout(() => r(), 10)); });', "sleep"],
      ['it("x", async () => { await sleep(10); });', "sleep"],
      ['it("x", async () => { while (!done()) { await new Promise((r) => setTimeout(r, 10)); } });', "sleep"],
      ['it("x", async () => { for (const step of steps) { await sleep(10); } });', "sleep"],
      ['it("x", async () => { for (let i = 0; i < 1; i++) { await sleep(30000); break; } });', "sleep"],
      ['it("x", async () => { for (let i = 0; i < 1; i++) { await sleep(30000); return; } });', "sleep"],
      ['it("x", async () => { for (let i = 0; i < 1; i++) { await sleep(30000); { break; } } });', "sleep"],
      ['it("x", async () => { while (true) { if (Date.now() > 0) {} await sleep(30000); break; } });', "sleep"],
      ['it("x", async () => { for (let i = 0; i < 1; i++) { await sleep(30000); if (true) break; } });', "sleep"],
      ['it("x", async () => { for (let i = 0; i < 1; i++) { await sleep(30000); true && (() => {})(); return; } });', "sleep"],
    ],
    pass: [
      "// @vitest-environment jsdom\nexport {};",
      'const view = <p title="// no">// text, not a comment</p>;',
      'it("x", async () => { await Promise.race([work, new Promise((_, reject) => setTimeout(reject, 5000))]); });',
      'it("x", async () => { for (let i = 0; i < 50; i++) { if (done()) break; await new Promise((r) => setTimeout(r, 10)); } });',
      'it("x", async () => { while (!done()) { if (Date.now() > end) throw new Error("late"); await sleep(10); } });',
      'it.each([1, 2])("x %d", (n) => { expect(n).toBeTruthy(); });',
    ],
  },
  {
    path: "scripts/tool.py",
    fail: [
      ["x = 1  # trailing", "comment"],
      ['def f():\n    """What f does."""\n    return 1', "comment"],
      ['"""Module docstring."""\nx = 1', "comment"],
      ["import os  # noqa: F401", "noqa"],
      ["x: int = 'a'  # type: ignore", "type: ignore"],
      ['import unittest\n@unittest.skip("later")\ndef test_a():\n    pass', "@unittest.skip"],
      ["import time\ntime.sleep(1)", "sleep"],
      ["import time\nwhile not ready():\n    time.sleep(1)", "sleep"],
      ["import time\nfor _ in range(1):\n    time.sleep(30)\n    break", "sleep"],
      ["import time\ndef f():\n    for _ in range(1):\n        time.sleep(30)\n        return", "sleep"],
      ["import time\nfor _ in range(1):\n    time.sleep(30)\n    with open(p):\n        break", "sleep"],
      ["import time\nwhile True:\n    time.monotonic()\n    time.sleep(30)\n    break", "sleep"],
      ["import time\nfor _ in range(1):\n    time.sleep(30)\n    if True:\n        break", "sleep"],
    ],
    pass: [
      "#!/usr/bin/env python3\nx = 1",
      "# -*- coding: utf-8 -*-\nx = 1",
      's = "# not a comment"\nt = """# nor this"""',
      "x = 'a' + '''b'''",
      "import time\nfor _ in range(50):\n    if ready():\n        break\n    time.sleep(1)",
      "import time\nwhile not ready():\n    if time.monotonic() > deadline:\n        raise TimeoutError\n    time.sleep(1)",
    ],
  },
  {
    path: "scripts/run.sh",
    fail: [
      ["echo hi # trailing", "comment"],
      ["# on its own line\necho hi", "comment"],
      ["# shellcheck disable=SC2086\necho $x", "shellcheck disable"],
      ["start_server\nsleep 2\ncurl localhost", "sleep"],
      ["make && sleep 1", "sleep"],
      ["until curl -sf localhost; do\n  sleep 1\ndone", "sleep"],
      ["while true; do\n  date\n  sleep 1\ndone", "sleep"],
      ["for _ in 1; do\n  sleep 30\n  break\ndone", "sleep"],
      ["for _ in 1; do sleep 30; exit 0; done", "sleep"],
      ["for _ in 1; do\n  sleep 30\n  if true; then break; fi\ndone", "sleep"],
      ["for _ in 1; do\n  sleep 30\n  true && break\ndone", "sleep"],
      ["for _ in 1; do\n  if true; then\n    echo\n  fi\n  sleep 30\n  break\ndone", "sleep"],
    ],
    pass: [
      "#!/usr/bin/env bash\necho hi",
      "# shellcheck source=scripts/versions.sh\n. scripts/versions.sh",
      "echo \"# no\" '# no' " + INTERPOLATE + "#x} a#b $((16#ff)) $'# no'",
      "cat <<EOF\n# inside a heredoc\nEOF\necho done",
      "for _ in $(seq 30); do\n  curl -sf localhost && break\n  sleep 1\ndone",
      "while ! curl -sf localhost; do\n  [ $SECONDS -lt 30 ] || exit 1\n  sleep 1\ndone",
      "for _ in $(seq 30); do\n  if curl -sf localhost; then\n    break\n  fi\n  sleep 1\ndone",
      "for _ in $(seq 30); do\n  case $(state) in\n    up) exit 0 ;;\n  esac\n  sleep 1\ndone",
      "echo \"$(printf '%s' \"# no\")\"",
    ],
  },
  {
    path: "scripts/probe/system.swift",
    fail: [
      ["let a = 1 // trailing", "comment"],
      ["/// what f does\nfunc f() {}", "doc comment"],
      ["/* a /* nested */ b */ let a = 1", "comment"],
      ["sleep(1)", "sleep"],
      ["while !ready() { usleep(1000) }", "sleep"],
      ["for _ in 0..<1 { usleep(30_000_000); break }", "sleep"],
      ["for _ in 0..<1 { usleep(30_000_000); return }", "sleep"],
      ["for _ in 0..<1 { usleep(30_000_000); if true { break } }", "sleep"],
    ],
    pass: [
      'let s = "// no /* no */"',
      'let r = #"// "raw" no"#',
      'let m = """\n// no\n"""',
      'let i = "a \\(b + "// no") c"',
      "for _ in 0..<50 { if ready() { break }; usleep(1000) }",
      "for _ in 0..<50 { guard !ready() else { return }; usleep(1000) }",
    ],
  },
  {
    path: ".github/workflows/ci.yml",
    fail: [
      ["on: push # trailing", "comment"],
      ["# own line\non: push", "comment"],
      ["steps:\n  - uses: actions/checkout@v7 # v7.0.1", "comment"],
      ["jobs:\n  a:\n    steps:\n      - run: |\n          echo hi # in the script\n", "comment"],
      ["jobs:\n  a:\n    steps:\n      - run: |\n          # shellcheck disable=SC2046\n          echo $(x)\n", "shellcheck disable"],
    ],
    pass: [
      "steps:\n  - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1",
      'name: "a # no"', "name: 'it''s # no'", "url: http://x#fragment", "body: |\n  # data, not a comment\nnext: 1"],
  },
  {
    path: "Cargo.toml",
    fail: [
      ['name = "a" # trailing', "comment"],
      ['[lints.clippy]\nfoo = "allow"', "lints allow"],
      ['[workspace.lints.rust]\nunused = { level = "allow", priority = 1 }', "lints allow"],
    ],
    pass: ["name = \"# no\"\ndesc = '''# no'''", '[lints.rust]\nunsafe_code = "deny"', '[features]\nallow = ["allow"]'],
  },
  {
    path: "Dockerfile",
    fail: [
      ["FROM a\n# own line\nRUN b", "comment"],
      ["FROM a\nRUN echo hi # trailing", "comment"],
      ["FROM a\nRUN make && \\\n    sleep 3", "sleep"],
    ],
    pass: ["# syntax=docker/dockerfile:1.7\nFROM a", 'FROM a\nRUN echo "# no"\nENV A="#x"', 'FROM a\nCMD ["sh", "-c", "echo # no"]'],
  },
  {
    path: "src/styles.css",
    fail: [["a { color: red; } /* why */", "comment"]],
    pass: ['a::before { content: "/* no */"; background: url(http://x/a.png); }'],
  },
  {
    path: "index.html",
    fail: [
      ["<p>a</p><!-- why -->", "comment"],
      ["<script>const a = 1; // why</script>", "comment"],
      ["<style>a { color: red; } /* why */</style>", "comment"],
    ],
    pass: ['<p title="<!-- no -->">// text</p>', '<script type="application/json">{"a": "//"}</script>'],
  },
  {
    path: "migrations/0001.sql",
    fail: [
      ["SELECT 1; -- why", "comment"],
      ["/* why */ SELECT 1;", "comment"],
    ],
    pass: ["SELECT '-- no', \"a--b\", '/* no */';"],
  },
  {
    path: "tsconfig.json",
    fail: [['{\n  // why\n  "a": 1\n}', "comment"]],
    pass: ['{"a": "http://x//y", "b": "/* no */"}'],
  },
  {
    path: ".gitignore",
    fail: [["# why\nnode_modules/", "comment"]],
    pass: ["node_modules/\n\\#literal"],
  },
  {
    path: "formal/Model.tla",
    fail: [
      ["x == 1 \\* why", "comment"],
      ["(* why (* nested *) *)\nx == 1", "comment"],
    ],
    pass: ['x == a /\\ b \\/ c\ny == "\\* no"'],
  },
  {
    path: "release-keys.pub",
    fail: [["# why this key\nAAAAC3NzaC1lZDI1NTE5", "comment"]],
    pass: ["AAAAC3NzaC1lZDI1NTE5\nAAAAC3NzaC1lZDI1NTE6"],
  },
  {
    path: "public/_headers",
    fail: [["# why\n/*\n  X-Frame-Options: DENY", "comment"]],
    pass: ["/*\n  X-Frame-Options: DENY"],
  },
];

describe("each language", () => {
  for (const language of LANGUAGES) {
    describe(language.path, () => {
      for (const [source, what] of language.fail) {
        it("fails on " + JSON.stringify(source).slice(0, 70), () => {
          const units = found(language.path, source);
          assert.ok(
            units.some((unit) => unit.what === what),
            "expected " + what + ", got [" + units.map((unit) => unit.what).join(", ") + "]",
          );
        });
      }
      for (const source of language.pass) {
        it("passes " + JSON.stringify(source).slice(0, 70), () => {
          assert.deepEqual(
            found(language.path, source).map((unit) => unit.what + ": " + unit.raw),
            [],
          );
        });
      }
    });
  }
  it("joins a run of line comments into one comment", () => {
    const units = found("src/a.ts", "// one\n// two\n// three\nconst a = 1;");
    assert.deepEqual(
      units.map((unit) => [unit.line, unit.endLine]),
      [[1, 3]],
    );
  });
  it("hashes a comment by its text, not its indentation", () => {
    const [flat] = found("src/a.ts", "// keep\nconst a = 1;");
    const [deep] = found("src/a.ts", "function f() {\n        // keep\n}");
    assert.equal(flat.sha256, deep.sha256);
  });
});

function git(cwd, ...args) {
  return execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["pipe", "pipe", "pipe"] }).trim();
}

function write(dir, files) {
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(dir, path)), { recursive: true });
    writeFileSync(join(dir, path), text);
  }
}

function repoWith(files) {
  const dir = mkdtempSync(join(tmpdir(), "grants-"));
  git(dir, "init", "-q", "-b", "main");
  git(dir, "config", "user.email", "test@example.com");
  git(dir, "config", "user.name", "test");
  write(dir, files);
  git(dir, "add", "-A");
  git(dir, "commit", "-qm", "base");
  const ledger = {
    existing: { commit: git(dir, "rev-parse", "HEAD"), files: Object.keys(files).filter((path) => PROSE.test(path)) },
    upstream: [{ path: "vendor/up/", except: ["ours.rs"], reason: "test" }],
    grants: [],
  };
  write(dir, { "grants.json": JSON.stringify(ledger, null, 2) + "\n" });
  git(dir, "add", "-A");
  git(dir, "commit", "-qm", "ledger");
  git(dir, "checkout", "-qb", "change");
  return { dir, base: git(dir, "rev-parse", "HEAD") };
}

function shipMigrations(repo, files) {
  setLedger(repo.dir, (ledger) => {
    ledger.migrations = { commit: git(repo.dir, "rev-parse", "HEAD"), files };
  });
  commitAll(repo, "ship migrations");
}

function setLedger(dir, change) {
  const ledger = JSON.parse(readFileSync(join(dir, "grants.json"), "utf8"));
  change(ledger);
  writeFileSync(join(dir, "grants.json"), JSON.stringify(ledger, null, 2) + "\n");
}

function commitAll(repo, message) {
  git(repo.dir, "add", "-A");
  git(repo.dir, "commit", "-qm", message);
  repo.base = git(repo.dir, "rev-parse", "HEAD");
}

function scenario(files, body) {
  return () => {
    const repo = repoWith(files);
    try {
      body(repo, () => check(repo.dir, { base: repo.base }));
    } finally {
      rmSync(repo.dir, { recursive: true, force: true });
    }
  };
}

const SOURCE = "export function a() {\n  // an old comment\n  return 1;\n}\n";
const approvalOf = (repo) => "grants-approved-" + hashText(readFileSync(join(repo.dir, "grants.json"), "utf8")).slice(0, 12);
const grantOf = (kind, path, text, words = "keep it") => ({ kind, path, sha256: hashText(text), words, date: "2026-11-17" });

describe("a change", () => {
  it(
    "fails on a comment it adds",
    scenario({ "src/a.ts": SOURCE }, (repo, run) => {
      write(repo.dir, { "src/a.ts": SOURCE + "// new reason\nexport const b = 2;\n" });
      assert.deepEqual(
        run().findings.map((f) => [f.path, f.line, f.what]),
        [["src/a.ts", 5, "comment"]],
      );
    }),
  );
  it(
    "fails on an old comment whose line it modifies",
    scenario({ "src/a.ts": SOURCE }, (repo, run) => {
      write(repo.dir, { "src/a.ts": SOURCE.replace("an old comment", "an edited comment") });
      assert.deepEqual(
        run().findings.map((f) => f.line),
        [2],
      );
    }),
  );
  it(
    "fails on a comment in a file it creates and has not committed",
    scenario({ "src/a.ts": SOURCE }, (repo, run) => {
      write(repo.dir, { "src/b.ts": "// fresh\nexport {};\n" });
      assert.deepEqual(
        run().findings.map((f) => f.path),
        ["src/b.ts"],
      );
    }),
  );
  it(
    "fails on a new prose file",
    scenario({ "README.md": "# A\n" }, (repo, run) => {
      write(repo.dir, { "docs/NOTES.md": "# notes\n", "notes.txt": "x\n" });
      assert.deepEqual(run().findings.map((f) => [f.path, f.kind]).sort(), [
        ["docs/NOTES.md", "document"],
        ["notes.txt", "document"],
      ]);
    }),
  );
  for (const [path, text] of [
    ["docs/NOTES", "Remember that the cache is cold on boot.\n"],
    ["docs/notes.org", "* Why\nThe cache is cold on boot.\n"],
    ["scripts/zz.xyz", "// a comment\n"],
    ["Makefile.zz", "# a comment\n"],
    ["docs/design.html", "<p>The cache is cold on boot because nothing warms it.</p>\n"],
  ]) {
    it(
      "fails on a new " + path + " instead of passing it unread",
      scenario({ "src/a.ts": SOURCE }, (repo, run) => {
        write(repo.dir, { [path]: text });
        assert.deepEqual(
          run().findings.map((f) => f.path),
          [path],
        );
      }),
    );
  }
  it(
    "fails on a file of an unknown type committed on the branch, and names both fixes",
    scenario({ "src/a.ts": SOURCE }, (repo) => {
      write(repo.dir, { "scripts/zz.xyz": "// a comment\n" });
      git(repo.dir, "add", "-A");
      git(repo.dir, "commit", "-qm", "add");
      const result = check(repo.dir, { base: repo.base, head: "HEAD" });
      assert.deepEqual(
        result.findings.map((f) => [f.path, f.kind]),
        [["scripts/zz.xyz", "unread"]],
      );
      const printed = report(result, { rev: repo.base, label: "test" });
      assert.match(printed, /scripts\/zz\.xyz/);
      assert.match(printed, /add a reader/i);
      assert.match(printed, /DATA/);
      assert.doesNotMatch(printed, / grant /);
    }),
  );
  it(
    "fails on a change to a file of an unknown type already in the tree",
    scenario({ "src/a.ts": SOURCE, "docs/OLD.xyz": "old\n" }, (repo, run) => {
      write(repo.dir, { "docs/OLD.xyz": "old\nnew words\n" });
      assert.deepEqual(
        run().findings.map((f) => [f.path, f.kind]),
        [["docs/OLD.xyz", "unread"]],
      );
    }),
  );
  it(
    "fails on a file of an unknown type renamed into place",
    scenario({ "src/a.ts": SOURCE, "docs/OLD.xyz": "old words that stay the same\n" }, (repo, run) => {
      git(repo.dir, "mv", "docs/OLD.xyz", "docs/NOTES");
      assert.deepEqual(
        run().findings.map((f) => [f.path, f.kind]),
        [["docs/NOTES", "unread"]],
      );
    }),
  );
  it(
    "passes an untouched file of an unknown type, a data file, a binary file and an empty file",
    scenario({ "src/a.ts": SOURCE, "docs/OLD.xyz": "// old\n", "Cargo.lock": "version = 4\n" }, (repo, run) => {
      write(repo.dir, {
        "src/a.ts": SOURCE + "export const b = 2;\n",
        "Cargo.lock": "version = 4\n[[package]]\n",
        "assets/blob.bin": "a\u0000b",
        "assets/.gitkeep": "",
      });
      const result = run();
      assert.deepEqual([...result.findings, ...result.problems], []);
    }),
  );
  it(
    "reads a symbolic link as a link, not as the file or folder it points to",
    scenario({ "src/a.ts": SOURCE }, (repo) => {
      symlinkSync("src", join(repo.dir, "code"));
      symlinkSync("src/a.ts", join(repo.dir, "AGENTS"));
      const untracked = check(repo.dir, { base: repo.base });
      assert.deepEqual([...untracked.findings, ...untracked.problems], []);
      git(repo.dir, "add", "-A");
      git(repo.dir, "commit", "-qm", "links");
      const committed = check(repo.dir, { base: repo.base, head: "HEAD" });
      assert.deepEqual([...committed.findings, ...committed.problems], []);
    }),
  );
  it(
    "lets a change edit an existing HTML page and refuses a new one",
    scenario({ "src/a.ts": SOURCE, "index.html": "<p>old</p>\n" }, (repo, run) => {
      write(repo.dir, { "index.html": "<p>new</p>\n", "page.html": "<p>new</p>\n" });
      assert.deepEqual(
        run().findings.map((f) => [f.path, f.kind]),
        [["page.html", "document"]],
      );
    }),
  );
  it(
    "fails on a prose file added to the existing list after the fact",
    scenario({ "README.md": "# A\n" }, (repo, run) => {
      write(repo.dir, { "LATER.md": "# later\n" });
      setLedger(repo.dir, (ledger) => ledger.existing.files.push("LATER.md"));
      assert.match(
        run()
          .problems.map((p) => p.message)
          .join("\n"),
        /LATER\.md is listed as existing but was not a file at/,
      );
    }),
  );
  it(
    "fails on a granted comment whose text changed, and on the grant it leaves unused",
    scenario({ "src/a.ts": SOURCE }, (repo, run) => {
      setLedger(repo.dir, (ledger) => ledger.grants.push(grantOf("comment", "src/a.ts", "// an old comment")));
      write(repo.dir, { "src/a.ts": SOURCE.replace("an old comment", "a reworded comment") });
      const result = run();
      assert.deepEqual(
        result.findings.map((f) => f.line),
        [2],
      );
      assert.match(result.problems.map((p) => p.message).join("\n"), /comment grant for src\/a\.ts .* matches nothing/);
    }),
  );
  it(
    "fails on a grant that matches nothing",
    scenario({ "src/a.ts": SOURCE, "README.md": "# A\n" }, (repo, run) => {
      setLedger(repo.dir, (ledger) => {
        ledger.grants.push(grantOf("suppression", "src/a.ts", "nothing"));
        ledger.grants.push({ kind: "document", path: "GONE.md", words: "w", date: "2026-11-17" });
      });
      const messages = run()
        .problems.map((p) => p.message)
        .join("\n");
      assert.match(messages, /suppression grant for src\/a\.ts .* matches nothing/);
      assert.match(messages, /document grant for GONE\.md matches no file/);
    }),
  );
  it(
    "fails on a grant without the owner's words",
    scenario({ "src/a.ts": SOURCE }, (repo, run) => {
      setLedger(repo.dir, (ledger) => ledger.grants.push(grantOf("comment", "src/a.ts", "// an old comment", "")));
      assert.match(
        run()
          .problems.map((p) => p.message)
          .join("\n"),
        /has no words from the owner/,
      );
    }),
  );
  it(
    "prints a grant it adds, with the text it grants",
    scenario({ "src/a.ts": SOURCE }, (repo, run) => {
      write(repo.dir, { "src/a.ts": SOURCE + "// granted reason\nexport const b = 2;\n" });
      setLedger(repo.dir, (ledger) => ledger.grants.push(grantOf("comment", "src/a.ts", "// granted reason", "I said so")));
      const result = check(repo.dir, { base: repo.base, approvals: [approvalOf(repo)] });
      assert.deepEqual([...result.findings, ...result.problems], []);
      const printed = report(result, { rev: repo.base, label: "test" });
      assert.match(printed, /GRANTS CHANGED/);
      assert.match(printed, /\+ added {4}comment in src\/a\.ts:5, 2026-11-17, "I said so"\n {6}\/\/ granted reason/);
    }),
  );
  it(
    "fails on a ledger change nobody approved, and names the approval and who may give it",
    scenario({ "src/a.ts": SOURCE }, (repo, run) => {
      setLedger(repo.dir, (ledger) => ledger.grants.push(grantOf("comment", "src/a.ts", "// an old comment")));
      const result = run();
      assert.deepEqual(result.findings, []);
      const printed = report(result, { rev: repo.base, label: "test" });
      assert.match(printed, new RegExp("grants\\.json .*" + approvalOf(repo)));
      assert.match(printed, /report the exact text/);
      assert.match(printed, /never approves their own/);
    }),
  );
  it(
    "fails on a new ledger nobody approved",
    scenario({ "src/a.ts": SOURCE }, (repo) => {
      const result = check(repo.dir, { base: git(repo.dir, "rev-parse", "HEAD~1") });
      assert.match(
        result.problems.map((p) => p.message).join("\n"),
        new RegExp(approvalOf(repo)),
      );
    }),
  );
  it(
    "passes a ledger change approved for exactly that ledger",
    scenario({ "src/a.ts": SOURCE }, (repo) => {
      setLedger(repo.dir, (ledger) => ledger.grants.push(grantOf("comment", "src/a.ts", "// an old comment")));
      const result = check(repo.dir, { base: repo.base, approvals: ["grants-approved-000000000000", approvalOf(repo)] });
      assert.deepEqual([...result.findings, ...result.problems], []);
    }),
  );
  it(
    "fails again when the ledger changes after its approval",
    scenario({ "src/a.ts": SOURCE }, (repo) => {
      setLedger(repo.dir, (ledger) => ledger.grants.push(grantOf("comment", "src/a.ts", "// an old comment")));
      const approved = approvalOf(repo);
      setLedger(repo.dir, (ledger) => {
        ledger.grants[0].words = "reworded after the approval";
      });
      const result = check(repo.dir, { base: repo.base, approvals: [approved] });
      assert.equal(result.problems.length, 1);
      assert.match(result.problems[0].message, new RegExp(approvalOf(repo)));
    }),
  );
  it(
    "fails on a committed ledger change nobody approved",
    scenario({ "src/a.ts": SOURCE }, (repo) => {
      setLedger(repo.dir, (ledger) => ledger.grants.push(grantOf("comment", "src/a.ts", "// an old comment")));
      const label = approvalOf(repo);
      git(repo.dir, "add", "-A");
      git(repo.dir, "commit", "-qm", "grant");
      assert.equal(check(repo.dir, { base: repo.base, head: "HEAD" }).problems.length, 1);
      assert.deepEqual(check(repo.dir, { base: repo.base, head: "HEAD", approvals: [label] }).problems, []);
    }),
  );
  it(
    "prints a grant it removes or rewords",
    scenario({ "src/a.ts": SOURCE }, (repo, run) => {
      write(repo.dir, { "src/a.ts": SOURCE + "// an old comment\n" });
      const grant = grantOf("comment", "src/a.ts", "// an old comment", "first");
      setLedger(repo.dir, (ledger) => ledger.grants.push(grant, { ...grant }));
      commitAll(repo, "grants");
      setLedger(repo.dir, (ledger) => {
        ledger.grants = [{ ...grant, words: "second" }];
      });
      const changes = run().ledgerChanges.join("\n");
      assert.match(changes, /- removed {2}comment in src\/a\.ts/);
      assert.match(changes, /~ altered {2}.*"first"[\s\S]*now .*"second"/);
    }),
  );
  it(
    "passes a granted comment after unrelated edits move it",
    scenario({ "src/a.ts": SOURCE }, (repo, run) => {
      write(repo.dir, { "src/a.ts": SOURCE + "// granted reason\nexport const b = 2;\n" });
      setLedger(repo.dir, (ledger) => ledger.grants.push(grantOf("comment", "src/a.ts", "// granted reason")));
      commitAll(repo, "granted");
      write(repo.dir, { "src/a.ts": "export const z = 0;\n" + SOURCE + "// granted reason\nexport const b = 3;\n" });
      const result = run();
      assert.deepEqual([...result.findings, ...result.problems], []);
    }),
  );
  it(
    "passes a renamed file and a moved prose file",
    scenario({ "src/a.ts": SOURCE, "docs/OLD.md": "# old\n" }, (repo, run) => {
      mkdirSync(join(repo.dir, "lib"), { recursive: true });
      git(repo.dir, "mv", "src/a.ts", "lib/a.ts");
      git(repo.dir, "mv", "docs/OLD.md", "docs/NEW.md");
      const result = run();
      assert.deepEqual([...result.findings, ...result.problems], []);
    }),
  );
  it(
    "passes an edit to an existing prose file and to code beside an old comment",
    scenario({ "src/a.ts": SOURCE, "README.md": "# A\n" }, (repo, run) => {
      write(repo.dir, { "README.md": "# A\n\nMore.\n", "src/a.ts": SOURCE.replace("return 1", "return 2") });
      const result = run();
      assert.deepEqual([...result.findings, ...result.problems], []);
    }),
  );
  it(
    "passes a whitespace-only change to a line with an old comment",
    scenario({ "src/a.ts": SOURCE }, (repo, run) => {
      write(repo.dir, { "src/a.ts": SOURCE.replace("  // an old", "    // an old") });
      assert.deepEqual(run().findings, []);
    }),
  );
  it(
    "passes upstream code the ledger names, and not the files it excepts",
    scenario({ "vendor/up/lib.rs": "fn a() {}\n", "vendor/up/ours.rs": "fn a() {}\n" }, (repo, run) => {
      write(repo.dir, {
        "vendor/up/lib.rs": "// upstream\nfn a() {}\n",
        "vendor/up/CHANGELOG.md": "upstream notes\n",
        "vendor/up/ours.rs": "// ours\nfn a() {}\n",
      });
      assert.deepEqual(
        run().findings.map((f) => f.path),
        ["vendor/up/ours.rs"],
      );
    }),
  );
});

function cli(repo, args, env = {}) {
  return spawnSync("node", [CLI, ...args], { cwd: repo.dir, encoding: "utf8", env: { ...process.env, GITHUB_ACTIONS: "", ...env } });
}

describe("the command", () => {
  it(
    "exits 1 and names the fix, the file and the line",
    scenario({ "src/a.ts": SOURCE }, (repo) => {
      write(repo.dir, { "src/a.ts": SOURCE + "// we retry because it flakes\n", "docs/X.md": "x\n" });
      const out = cli(repo, ["--base", repo.base]);
      assert.equal(out.status, 1);
      assert.match(out.stdout, /src\/a\.ts:5 {2}comment {2}\/\/ we retry because it flakes/);
      assert.match(out.stdout, /docs\/X\.md {2}new document/);
      assert.match(out.stdout, /encode it as a type, an assertion or a test/);
      assert.match(out.stdout, /node \S*check-grants\.mjs grant <file>:<line> --words/);
    }),
  );
  it(
    "compares a push to main with the commit before it",
    scenario({ "src/a.ts": SOURCE }, (repo) => {
      git(repo.dir, "checkout", "-q", "main");
      write(repo.dir, { "src/a.ts": SOURCE + "// on main\n" });
      commitAll(repo, "push");
      const out = cli(repo, [], { GITHUB_BASE_REF: "", GRANTS_BASE: "", GITHUB_REF: "" });
      assert.equal(out.status, 1);
      assert.match(out.stdout, /the previous commit on main/);
    }),
  );
  it(
    "fails on a ledger change in a local run, and passes it with the approval",
    scenario({ "src/a.ts": SOURCE }, (repo) => {
      setLedger(repo.dir, (ledger) => ledger.grants.push(grantOf("comment", "src/a.ts", "// an old comment")));
      const refused = cli(repo, ["--base", repo.base]);
      assert.equal(refused.status, 1, refused.stdout);
      assert.match(refused.stdout, new RegExp(approvalOf(repo)));
      const approved = cli(repo, ["--base", repo.base], { GRANTS_APPROVALS: approvalOf(repo) });
      assert.equal(approved.status, 0, approved.stdout);
    }),
  );
  it(
    "prints a ledger change on a push to main and passes it",
    scenario({ "src/a.ts": SOURCE }, (repo) => {
      git(repo.dir, "checkout", "-q", "main");
      setLedger(repo.dir, (ledger) => ledger.grants.push(grantOf("comment", "src/a.ts", "// an old comment")));
      commitAll(repo, "grant");
      const pushed = cli(repo, [], { GITHUB_BASE_REF: "", GRANTS_BASE: "", GITHUB_EVENT_NAME: "push", GITHUB_REF: "refs/heads/main" });
      assert.equal(pushed.status, 0, pushed.stdout);
      assert.match(pushed.stdout, /GRANTS CHANGED/);
      const local = cli(repo, [], { GITHUB_BASE_REF: "", GRANTS_BASE: "", GITHUB_EVENT_NAME: "", GITHUB_REF: "" });
      assert.equal(local.status, 1, local.stdout);
    }),
  );
  it(
    "records a grant that then covers exactly that comment",
    scenario({ "src/a.ts": SOURCE }, (repo) => {
      write(repo.dir, { "src/a.ts": SOURCE + "// the owner wants this\n" });
      const granted = cli(repo, ["grant", "src/a.ts:5", "--words", "keep it", "--date", "2026-11-17"]);
      assert.equal(granted.status, 0, granted.stderr);
      const out = cli(repo, ["--base", repo.base], { GRANTS_APPROVALS: "other-label " + approvalOf(repo) });
      assert.equal(out.status, 0, out.stdout);
      assert.match(out.stdout, /GRANTS CHANGED[\s\S]*\/\/ the owner wants this/);
    }),
  );
});

describe("strip", () => {
  const stripped = (files, body) =>
    scenario(files, (repo) => {
      const result = sweep(repo.dir);
      body(repo, result, (path) => readFileSync(join(repo.dir, path), "utf8"));
    });
  it(
    "removes own-line, trailing and inline comments, and the blank line a header leaves",
    stripped(
      {
        "src/a.ts": "// header\n\nexport function a(x: number) {\n  // why\n  const b = x /* inline */ + 1; // trailing\n\n  // a section\n\n  return b;\n}\n",
      },
      (repo, result, read) => {
        assert.equal(read("src/a.ts"), "export function a(x: number) {\n  const b = x + 1;\n\n  return b;\n}\n");
        assert.deepEqual(
          result.removed.map((unit) => unit.line),
          [1, 4, 5, 5, 7],
        );
      },
    ),
  );
  it(
    "removes a JSX comment with the braces that held it",
    stripped({ "src/a.tsx": "export const A = () => (\n  <div>\n    {/* why */}\n    <p>a</p>\n  </div>\n);\n" }, (repo, result, read) => {
      assert.equal(read("src/a.tsx"), "export const A = () => (\n  <div>\n    <p>a</p>\n  </div>\n);\n");
    }),
  );
  it(
    "gives a Python body that was only a docstring the pass it needs",
    stripped({ "scripts/a.py": '"""Module."""\n\n\nclass Failed(Exception):\n    """Why."""\n\n\ndef f():\n    """Doc."""\n    return 1\n' }, (repo, result, read) => {
      assert.equal(read("scripts/a.py"), "class Failed(Exception):\n    pass\n\n\ndef f():\n    return 1\n");
    }),
  );
  it(
    "keeps lines a tool reads, suppressions, granted comments, prose and shipped migrations",
    stripped(
      {
        "scripts/a.sh": "#!/usr/bin/env bash\n# shellcheck disable=SC2086\necho $A\n",
        "src/b.ts": "// the owner wants this\nexport const b = 1;\n",
        "index.html": "<!-- a note -->\n<p>a</p>\n",
        "migrations/0000_init.sql": "-- why\nCREATE TABLE a (id INTEGER);\n",
      },
      (repo, result, read) => {
        assert.equal(read("scripts/a.sh"), "#!/usr/bin/env bash\n# shellcheck disable=SC2086\necho $A\n");
        assert.equal(read("index.html"), "<!-- a note -->\n<p>a</p>\n");
        assert.equal(read("migrations/0000_init.sql"), "CREATE TABLE a (id INTEGER);\n");
        write(repo.dir, { "migrations/0000_init.sql": "-- why\nCREATE TABLE a (id INTEGER);\n", "src/b.ts": "// the owner wants this\nexport const b = 1;\n" });
        shipMigrations(repo, ["migrations/0000_init.sql"]);
        setLedger(repo.dir, (ledger) => ledger.grants.push(grantOf("comment", "src/b.ts", "// the owner wants this")));
        assert.deepEqual(sweep(repo.dir).removed, []);
        assert.equal(read("src/b.ts"), "// the owner wants this\nexport const b = 1;\n");
        assert.equal(read("migrations/0000_init.sql"), "-- why\nCREATE TABLE a (id INTEGER);\n");
      },
    ),
  );
  it(
    "records what it removed, formats Rust with rustfmt, and leaves a tree the lint passes",
    scenario({ "src/a.rs": "fn a() -> u8 {\n    1\n}\n" }, (repo) => {
      write(repo.dir, { "src/a.rs": "/// Doc.\nenum E {\n    /// One.\n    A {\n        x: u8,\n    },\n}\n" });
      commitAll(repo, "comments");
      const record = join(repo.dir, "..", "removed-" + Date.now() + ".tsv");
      const out = cli(repo, ["strip", "--record", record]);
      assert.equal(out.status, 0, out.stderr);
      assert.equal(readFileSync(join(repo.dir, "src/a.rs"), "utf8"), "enum E {\n    A { x: u8 },\n}\n");
      assert.deepEqual(readFileSync(record, "utf8").trim().split("\n"), [
        "path\tline\tend_line\tlanguage\tkind\ttext",
        "src/a.rs\t1\t1\trust\tdoc comment\t/// Doc.",
        "src/a.rs\t3\t3\trust\tdoc comment\t/// One.",
      ]);
      rmSync(record);
      assert.deepEqual(check(repo.dir, { base: repo.base }).findings, []);
    }),
  );
});
