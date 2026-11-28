import { execFileSync } from "node:child_process";
import { createPublicKey, sign, verify } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join, resolve } from "node:path";
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

export function releaseFaults(tag, names, read, keys) {
  const faults = [];
  const has = (name) => {
    if (!names.includes(name)) faults.push(`The release has no ${name}.`);
    return names.includes(name);
  };
  const disks = names.filter((name) => name.endsWith(".dmg"));
  if (disks.length !== 1) faults.push(`The release has ${disks.length} disk images and needs exactly one.`);
  else if (has(`${disks[0]}.sha256`)) {
    const named = read(`${disks[0]}.sha256`).toString().trim().split("\n").map((line) => /^[a-f0-9]{64} [ *](.+)$/.exec(line)?.[1]);
    if (named.length !== 1 || named[0] !== disks[0]) faults.push(`The checksum file does not name ${disks[0]}, and only it.`);
  }
  const hasManifest = has("guaca-release.json"), hasSignature = has("guaca-release.json.sig");
  if (hasManifest && hasSignature) {
    const bytes = read("guaca-release.json");
    if (!trusted(bytes, read("guaca-release.json.sig").toString(), keys)) faults.push("The signature does not verify against release-keys.pub.");
    const version = tag.replace(/^v/, "");
    const said = (() => { try { return JSON.parse(bytes)?.version; } catch { return undefined; } })();
    if (said !== version) faults.push(`The manifest names ${said ?? "no version"}, not ${version}.`);
  }
  return faults;
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

const running = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (running && process.argv[2] === "--check") {
  const [tag, listing, directory] = process.argv.slice(3);
  if (!tag) throw new Error("Usage: node scripts/release-manifest.mjs --check TAG [listing.json directory]");
  const gh = (...args) => execFileSync("gh", [...args, "--repo", "madebywelch/guaca"], { encoding: "utf8" });
  const names = JSON.parse(listing ? readFileSync(listing, "utf8") : gh("release", "view", tag, "--json", "assets")).assets.map((asset) => asset.name);
  const dir = directory ?? mkdtempSync(join(tmpdir(), "guaca-release-check."));
  try {
    const small = names.filter((name) => !name.endsWith(".dmg"));
    if (!directory && small.length) gh("release", "download", tag, "--dir", dir, ...small.flatMap((name) => ["--pattern", name]));
    const faults = releaseFaults(tag, names, (name) => readFileSync(join(dir, name)), read("release-keys.pub"));
    for (const fault of faults) console.error(fault);
    if (faults.length) process.exitCode = 1;
    else console.log(`${tag} has the disk image, its checksum, guaca-release.json and its signature; the manifest verifies and names ${tag.replace(/^v/, "")}.\nBackend image: ${JSON.parse(readFileSync(join(dir, "guaca-release.json"))).image}`);
  } finally {
    if (!directory) rmSync(dir, { recursive: true, force: true });
  }
} else if (running) {
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
