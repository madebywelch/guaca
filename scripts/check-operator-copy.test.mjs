import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { test } from "node:test";
import { frontendDashes } from "./check-operator-copy.mjs";

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
