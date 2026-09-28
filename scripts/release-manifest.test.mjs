import { strict as assert } from "node:assert";
import { generateKeyPairSync } from "node:crypto";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { envelope, manifest, sharedVersion, signature, trusted } from "./release-manifest.mjs";
const commit = "a".repeat(40), image = `ghcr.io/madebywelch/guaca/guacad@sha256:${"b".repeat(64)}`;
const protocol = {generation: 1, minimum: 1, maximum: 1};
test("release metadata refuses mutable images, dirty commits, and incompatible ranges", () => {
  assert.throws(() => manifest("0.2.0", commit, "guacad:latest", protocol));
  assert.throws(() => manifest("0.2.0", `${commit}-dirty`, image, protocol));
  assert.throws(() => manifest("0.2.0-beta.1", commit, image, protocol));
  assert.throws(() => manifest("0.2.0", commit, image, {...protocol, maximum: 0}));
  assert.equal(manifest("0.2.0", commit, image, protocol).notes, "https://github.com/madebywelch/guaca/releases/tag/v0.2.0");
});
test("the page, the desktop and the host are built at one version", () => {
  assert.match(sharedVersion(), /^\d+\.\d+\.\d+$/);
});
test("a manifest is signed over its exact bytes, and only a listed key is trusted", () => {
  const pair = () => {
    const { privateKey, publicKey } = generateKeyPairSync("ed25519");
    const raw = publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("base64");
    return { pem: privateKey.export({ format: "pem", type: "pkcs8" }), raw };
  };
  const ours = pair(), theirs = pair();
  const bytes = Buffer.from(`${JSON.stringify(manifest("0.2.0", commit, image, protocol), null, 2)}\n`);
  const said = signature(bytes, ours.pem);
  assert.ok(trusted(bytes, said, `# comment\n${theirs.raw}\n${ours.raw}\n`));
  assert.ok(!trusted(bytes, said, theirs.raw));
  assert.ok(!trusted(Buffer.concat([bytes, Buffer.from(" ")]), said, ours.raw));
});
const listed = (name) => readFileSync(new URL(`../${name}`, import.meta.url), "utf8")
  .split("\n").map((l) => l.trim()).filter((l) => l && !l.startsWith("#"));
test("the key lists the updater is built with are well formed, and share no key", () => {
  for (const name of ["release-keys.pub", "main-keys.pub"]) {
    const keys = listed(name);
    assert.ok(keys.length > 0, name);
    for (const key of keys) assert.equal(Buffer.from(key, "base64").length, 32, name);
  }
  const release = new Set(listed("release-keys.pub"));
  assert.ok(listed("main-keys.pub").every((key) => !release.has(key)), "a key CI holds must not sign releases");
});
test("a build of main names its channel and links its commit; a release names neither", () => {
  const build = manifest("0.2.0", commit, image, protocol, "main");
  assert.equal(build.channel, "main");
  assert.equal(build.notes, `https://github.com/madebywelch/guaca/commit/${commit}`);
  assert.ok(!("channel" in manifest("0.2.0", commit, image, protocol)), "older updaters refuse unknown fields");
  assert.throws(() => manifest("0.2.0", commit, image, protocol, "nightly"));
  assert.throws(() => manifest("0.2.0", `${commit}-dirty`, image, protocol, "main"));
});
test("main's envelope carries the exact bytes that were signed", () => {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const raw = publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("base64");
  const bytes = Buffer.from(`${JSON.stringify(manifest("0.2.0", commit, image, protocol, "main"), null, 2)}\n`);
  const said = signature(bytes, privateKey.export({ format: "pem", type: "pkcs8" }));
  const read = JSON.parse(envelope(bytes, said).toString());
  assert.deepEqual(Object.keys(read).sort(), ["manifest", "signature"]);
  const carried = Buffer.from(read.manifest, "base64");
  assert.ok(carried.equals(bytes));
  assert.ok(trusted(carried, read.signature, raw));
});
