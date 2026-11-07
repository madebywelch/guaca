import { strict as assert } from "node:assert";
import { execFileSync } from "node:child_process";
import { generateKeyPairSync, sign } from "node:crypto";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { publishedImage } from "./install-image.mjs";

test("verified history selects exact commits and refuses bad feeds", () => {
  const dir = mkdtempSync(join(tmpdir(), "guaca-feed-test-"));
  const git = (...args) => execFileSync("git", args, { cwd: dir, stdio: "pipe", encoding: "utf8" });
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const keys = publicKey.export({ type: "spki", format: "der" }).subarray(-32).toString("base64");
  const image = `ghcr.io/madebywelch/guaca/guacad@sha256:${"b".repeat(64)}`;
  const write = (commit, valid = true) => {
    const bytes = Buffer.from(JSON.stringify({ schema: 1, channel: "main", commit, image }));
    const signature = sign(null, valid ? bytes : Buffer.from("wrong"), privateKey).toString("base64");
    writeFileSync(join(dir, "guaca-main.json"), JSON.stringify({ manifest: bytes.toString("base64"), signature }));
    git("add", "guaca-main.json");
    git("commit", "--quiet", "-m", commit);
  };
  try {
    git("init", "--quiet", "-b", "main-feed");
    write("a".repeat(40));
    write("c".repeat(40));
    const lookup = commit => publishedImage({ source: dir, commit, keys });
    assert.equal(lookup("c".repeat(40)), image);
    assert.equal(lookup("a".repeat(40)), image);
    assert.equal(lookup("d".repeat(40)), "");
    assert.throws(() => publishedImage({ source: join(dir, "missing"), commit: "a".repeat(40), keys }));
    write("e".repeat(40), false);
    assert.throws(() => lookup("e".repeat(40)), /signature/);
    assert.throws(() => lookup("a".repeat(40)), /signature/);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
