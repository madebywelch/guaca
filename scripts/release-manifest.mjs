// Advisory release metadata. It never grants permission to install an image.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
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

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = fileURLToPath(new URL("../", import.meta.url));
  const git = (...args) => execFileSync("git", args, {cwd: root, encoding: "utf8"}).trim();
  if (git("status", "--porcelain")) throw new Error("Build release metadata from a clean checkout.");
  const read = name => readFileSync(resolve(root, name), "utf8");
  const { version } = JSON.parse(read("package.json"));
  const cargoVersion = read("src-tauri/Cargo.toml").match(/^version = "([^"]+)"/m)?.[1];
  if (JSON.parse(read("src-tauri/tauri.conf.json")).version !== version || cargoVersion !== version) throw new Error("Frontend, desktop and backend versions must match.");
  const value = manifest(version, git("rev-parse", "HEAD"), process.env.GUACA_BACKEND_IMAGE ?? "", JSON.parse(read("release-protocol.json")));
  if (!process.argv[2]) throw new Error("Usage: GUACA_BACKEND_IMAGE=... node scripts/release-manifest.mjs /path/to/guaca-release.json");
  writeFileSync(process.argv[2], `${JSON.stringify(value, null, 2)}\n`);
}
