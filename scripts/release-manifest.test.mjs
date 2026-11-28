import { strict as assert } from "node:assert";
import { spawnSync } from "node:child_process";
import { generateKeyPairSync } from "node:crypto";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { envelope, manifest, releaseFaults, sharedVersion, signature, trusted } from "./release-manifest.mjs";
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
const disk = "Guaca_0.2.0_universal.dmg";
const release = (version = "0.2.0") => {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  const bytes = Buffer.from(`${JSON.stringify(manifest(version, commit, image, protocol), null, 2)}\n`);
  const said = signature(bytes, privateKey.export({ format: "pem", type: "pkcs8" }));
  return {
    keys: publicKey.export({ format: "der", type: "spki" }).subarray(-32).toString("base64"),
    files: {
      [disk]: Buffer.from("disk image"),
      [`${disk}.sha256`]: Buffer.from(`${"c".repeat(64)}  ${disk}\n`),
      "guaca-release.json": bytes,
      "guaca-release.json.sig": Buffer.from(`${said}\n`),
    },
  };
};
const faultsOf = ({ files, keys }, tag = "v0.2.0", lacking = []) =>
  releaseFaults(tag, Object.keys(files).filter((name) => !lacking.includes(name)), (name) => files[name], keys);
test("a release that lacks any of its four files is refused, and the fault names the file", () => {
  const { files } = release();
  assert.match(faultsOf(release(), "v0.2.0", [disk]).join("\n"), /0 disk images/);
  assert.match(faultsOf(release(), "v0.2.0", [`${disk}.sha256`]).join("\n"), /no Guaca_0\.2\.0_universal\.dmg\.sha256/);
  assert.match(faultsOf(release(), "v0.2.0", ["guaca-release.json"]).join("\n"), /no guaca-release\.json\./);
  assert.match(faultsOf(release(), "v0.2.0", ["guaca-release.json.sig"]).join("\n"), /no guaca-release\.json\.sig/);
  assert.match(faultsOf({ files: { ...files, "Other.dmg": Buffer.from("") }, keys: release().keys }).join("\n"), /2 disk images/);
});
test("a checksum file must name the disk image, only it, in the form shasum writes", () => {
  for (const text of [`${"c".repeat(64)}  Guaca_0.1.0_universal.dmg\n`, `${"c".repeat(64)}  ${disk}\n${"d".repeat(64)}  Other.dmg\n`, `${disk}\n`, ""]) {
    const made = release();
    made.files[`${disk}.sha256`] = Buffer.from(text);
    assert.deepEqual(faultsOf(made), [`The checksum file does not name ${disk}, and only it.`], text);
  }
});
test("a manifest must verify against the listed keys and name the tag's version", () => {
  const made = release();
  assert.deepEqual(faultsOf(made, "v0.3.0"), ["The manifest names 0.2.0, not 0.3.0."]);
  assert.deepEqual(faultsOf({ ...made, keys: release().keys }), ["The signature does not verify against release-keys.pub."]);
  made.files["guaca-release.json.sig"] = Buffer.from("AAAA\n");
  assert.deepEqual(faultsOf(made), ["The signature does not verify against release-keys.pub."]);
  const edited = release();
  edited.files["guaca-release.json"] = Buffer.concat([edited.files["guaca-release.json"], Buffer.from(" ")]);
  assert.deepEqual(faultsOf(edited), ["The signature does not verify against release-keys.pub."]);
  const unreadable = release();
  unreadable.files["guaca-release.json"] = Buffer.from("not json");
  assert.equal(faultsOf(unreadable).length, 2);
});
test("a complete release has no faults", () => {
  assert.deepEqual(faultsOf(release()), []);
});
const checkout = (t) => {
  const dir = mkdtempSync(join(tmpdir(), "guaca-release-check-test-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  for (const sub of ["checkout/scripts", "bin", "download"]) mkdirSync(join(dir, sub), { recursive: true });
  copyFileSync(new URL("./release-manifest.mjs", import.meta.url), join(dir, "checkout/scripts/release-manifest.mjs"));
  const { files, keys } = release();
  writeFileSync(join(dir, "checkout/release-keys.pub"), `${keys}\n`);
  for (const [name, bytes] of Object.entries(files)) writeFileSync(join(dir, "download", name), bytes);
  const listing = (names) => {
    writeFileSync(join(dir, "listing.json"), JSON.stringify({ assets: names.map((name) => ({ name })) }));
    return join(dir, "listing.json");
  };
  const check = (args, env = {}) =>
    spawnSync(process.execPath, [join(dir, "checkout/scripts/release-manifest.mjs"), "--check", ...args], { encoding: "utf8", env });
  return { dir, listing, check, names: Object.keys(files) };
};
test("the check command reads a listing and files with no network, and exits nonzero on a fault", (t) => {
  const { dir, listing, check, names } = checkout(t);
  const lacking = check(["v0.2.0", listing(names.slice(0, 3)), join(dir, "download")]);
  assert.equal(lacking.status, 1);
  assert.equal(lacking.stderr, "The release has no guaca-release.json.sig.\n");
  const complete = check(["v0.2.0", listing(names), join(dir, "download")]);
  assert.equal(complete.status, 0, complete.stderr);
  assert.ok(complete.stdout.includes("verifies and names 0.2.0"));
  assert.ok(complete.stdout.includes(image));
  assert.equal(check(["v0.3.0", listing(names), join(dir, "download")]).status, 1);
  assert.equal(check([]).status, 1);
});
test("asked about a tag alone, the command lists the release with gh and downloads all but the disk image", (t) => {
  const { dir, listing, check, names } = checkout(t);
  const stand = join(dir, "bin/gh");
  writeFileSync(stand, `#!/bin/sh
echo "$@" >> "${dir}/calls"
[ "$2" = view ] && cat "${dir}/listing.json"
while [ $# -gt 0 ]; do [ "$1" = --dir ] && cp "${dir}/download"/* "$2"; shift; done
exit 0
`);
  chmodSync(stand, 0o755);
  listing(names);
  const checked = check(["v0.2.0"], { PATH: `${join(dir, "bin")}:/usr/bin:/bin` });
  assert.equal(checked.status, 0, checked.stderr);
  const calls = readFileSync(join(dir, "calls"), "utf8").split("\n").filter(Boolean);
  assert.equal(calls.length, 2);
  assert.equal(calls[0], "release view v0.2.0 --json assets --repo madebywelch/guaca");
  assert.match(calls[1], /^release download v0\.2\.0 --dir \S+ --pattern Guaca_0\.2\.0_universal\.dmg\.sha256 --pattern guaca-release\.json --pattern guaca-release\.json\.sig --repo madebywelch\/guaca$/);
});
