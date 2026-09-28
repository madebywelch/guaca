// Release metadata, and the signature that lets an updater install what it names.
import { execFileSync } from "node:child_process";
import { createPublicKey, sign, verify } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function manifest(version, commit, image, protocol) {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) throw new Error("A stable semantic release version is required.");
  if (!/^[a-f0-9]{40}$/.test(commit)) throw new Error("A full clean source commit is required.");
  if (!/^ghcr\.io\/madebywelch\/guaca\/guacad@sha256:[a-f0-9]{64}$/.test(image)) throw new Error("A published backend digest is required.");
  const {generation, minimum, maximum} = protocol;
  if (![generation, minimum, maximum].every(Number.isSafeInteger) || minimum < 1 || minimum > generation || maximum < generation) throw new Error("Invalid API compatibility range.");
  return {schema: 1, version, commit, image, apiGeneration: generation, clientMinimum: minimum, clientMaximum: maximum, notes: `https://github.com/madebywelch/guaca/releases/tag/v${version}`};
}

const root = fileURLToPath(new URL("../", import.meta.url));
const read = name => readFileSync(resolve(root, name), "utf8");

/** The one version all three builds carry. The page, the desktop and the host
 *  each report their own, and the host manager refuses an image whose label
 *  disagrees with the app, so a drift here is an update that cannot finish. */
export function sharedVersion() {
  const { version } = JSON.parse(read("package.json"));
  const cargoVersion = read("src-tauri/Cargo.toml").match(/^version = "([^"]+)"/m)?.[1];
  if (JSON.parse(read("src-tauri/tauri.conf.json")).version !== version || cargoVersion !== version) throw new Error("Frontend, desktop and backend versions must match.");
  return version;
}

/** Base64 Ed25519 signature over exactly the bytes that are published. */
export function signature(bytes, privateKeyPem) {
  return sign(null, bytes, privateKeyPem).toString("base64");
}

/** Whether a key in `keys` (release-keys.pub's format) made `said` over `bytes`.
 *  Checked before anything is written: a signature no updater accepts is a
 *  release every box refuses, found out on every box. */
export function trusted(bytes, said, keys) {
  const spki = Buffer.from("302a300506032b6570032100", "hex");
  return keys
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"))
    .some((line) => {
      const key = createPublicKey({ key: Buffer.concat([spki, Buffer.from(line, "base64")]), format: "der", type: "spki" });
      return verify(null, bytes, key, Buffer.from(said, "base64"));
    });
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const git = (...args) => execFileSync("git", args, {cwd: root, encoding: "utf8"}).trim();
  if (git("status", "--porcelain")) throw new Error("Build release metadata from a clean checkout.");
  const value = manifest(sharedVersion(), git("rev-parse", "HEAD"), process.env.GUACA_BACKEND_IMAGE ?? "", JSON.parse(read("release-protocol.json")));
  if (!process.argv[2]) throw new Error("Usage: GUACA_BACKEND_IMAGE=... node scripts/release-manifest.mjs /path/to/guaca-release.json");
  const bytes = Buffer.from(`${JSON.stringify(value, null, 2)}\n`);
  const key = readFileSync(process.env.GUACA_RELEASE_SIGNING_KEY ?? resolve(homedir(), ".config/guaca/release-signing-key.pem"), "utf8");
  const said = signature(bytes, key);
  if (!trusted(bytes, said, read("release-keys.pub"))) throw new Error("That signing key is not in release-keys.pub. No updater would install this release.");
  writeFileSync(process.argv[2], bytes);
  writeFileSync(`${process.argv[2]}.sig`, `${said}\n`);
}
