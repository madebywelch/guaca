import { execFileSync } from "node:child_process";
import { createPublicKey, sign, verify } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

export function manifest(version, commit, image, protocol, channel = "release") {
  if (!/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version)) throw new Error("A stable semantic release version is required.");
  if (!/^[a-f0-9]{40}$/.test(commit)) throw new Error("A full clean source commit is required.");
  if (!/^ghcr\.io\/madebywelch\/guaca\/guacad@sha256:[a-f0-9]{64}$/.test(image)) throw new Error("A published backend digest is required.");
  const {generation, minimum, maximum} = protocol;
  if (![generation, minimum, maximum].every(Number.isSafeInteger) || minimum < 1 || minimum > generation || maximum < generation) throw new Error("Invalid API compatibility range.");
  const range = {apiGeneration: generation, clientMinimum: minimum, clientMaximum: maximum};
  if (channel === "release") return {schema: 1, version, commit, image, ...range, notes: `https://github.com/madebywelch/guaca/releases/tag/v${version}`};
  if (channel === "main") return {schema: 1, channel, version, commit, image, ...range, notes: `https://github.com/madebywelch/guaca/commit/${commit}`};
  throw new Error("The channel is release or main.");
}

export function envelope(bytes, said) {
  return Buffer.from(`${JSON.stringify({manifest: bytes.toString("base64"), signature: said})}\n`);
}

const root = fileURLToPath(new URL("../", import.meta.url));
const read = name => readFileSync(resolve(root, name), "utf8");

export function sharedVersion() {
  const { version } = JSON.parse(read("package.json"));
  const cargoVersion = read("src-tauri/Cargo.toml").match(/^version = "([^"]+)"/m)?.[1];
  if (JSON.parse(read("src-tauri/tauri.conf.json")).version !== version || cargoVersion !== version) throw new Error("Frontend, desktop and backend versions must match.");
  return version;
}

export function signature(bytes, privateKeyPem) {
  return sign(null, bytes, privateKeyPem).toString("base64");
}

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

export function readMainEnvelope(raw, keys) {
  const { manifest, signature } = JSON.parse(raw);
  if (typeof manifest !== "string" || typeof signature !== "string") throw new Error("The main feed envelope is invalid.");
  const bytes = Buffer.from(manifest, "base64");
  if (!trusted(bytes, signature, keys)) throw new Error("The main feed signature is invalid.");
  const value = JSON.parse(bytes.toString("utf8"));
  if (value.schema !== 1 || value.channel !== "main" || !/^[a-f0-9]{40}$/.test(value.commit) || !/^ghcr\.io\/madebywelch\/guaca\/guacad@sha256:[a-f0-9]{64}$/.test(value.image)) throw new Error("The main feed metadata is invalid.");
  return value;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const main = args[0] === "--channel" && args[1] === "main";
  const out = main ? args[2] : args[0];
  if (!out || (args[0] === "--channel" && !main)) throw new Error("Usage: GUACA_BACKEND_IMAGE=... node scripts/release-manifest.mjs [--channel main] /path/to/manifest.json");
  const git = (...args) => execFileSync("git", args, {cwd: root, encoding: "utf8"}).trim();
  if (git("status", "--porcelain")) throw new Error("Build release metadata from a clean checkout.");
  const channel = main ? "main" : "release";
  const value = manifest(sharedVersion(), git("rev-parse", "HEAD"), process.env.GUACA_BACKEND_IMAGE ?? "", JSON.parse(read("release-protocol.json")), channel);
  const bytes = Buffer.from(`${JSON.stringify(value, null, 2)}\n`);
  const key = main
    ? process.env.GUACA_MAIN_SIGNING_KEY || readFileSync(resolve(homedir(), ".config/guaca/main-signing-key.pem"), "utf8")
    : readFileSync(process.env.GUACA_RELEASE_SIGNING_KEY ?? resolve(homedir(), ".config/guaca/release-signing-key.pem"), "utf8");
  const said = signature(bytes, key);
  const list = main ? "main-keys.pub" : "release-keys.pub";
  if (!trusted(bytes, said, read(list))) throw new Error(`That signing key is not in ${list}. No updater would install this ${main ? "build" : "release"}.`);
  if (main) {
    writeFileSync(out, envelope(bytes, said));
  } else {
    writeFileSync(out, bytes);
    writeFileSync(`${out}.sig`, `${said}\n`);
  }
}
