import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { readMainEnvelope } from "./release-manifest.mjs";

export function publishedImage({ source, commit, keys }) {
  if (!/^[a-f0-9]{40}$/.test(commit)) throw new Error("Invalid checkout commit.");
  const dir = mkdtempSync(join(tmpdir(), "guaca-feed-"));
  const git = (...args) => execFileSync("git", args, { cwd: dir, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], maxBuffer: 32 * 1024 * 1024 }).trim();
  try {
    git("init", "--bare", "--quiet");
    git("fetch", "--quiet", "--no-tags", source, "refs/heads/main-feed:refs/heads/main-feed");
    const revisions = git("rev-list", "refs/heads/main-feed").split("\n").filter(Boolean);
    if (!revisions.length) throw new Error("The main feed has no history.");
    for (const revision of revisions) {
      const value = readMainEnvelope(git("show", `${revision}:guaca-main.json`), keys);
      if (value.commit === commit) return value.image;
    }
    return "";
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === new URL(`file://${process.argv[1]}`).href) {
  try {
    const commit = execFileSync("git", ["rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    console.log(publishedImage({ source: "https://github.com/madebywelch/guaca.git", commit, keys: readFileSync("main-keys.pub", "utf8") }));
  } catch (error) {
    console.error(`Could not verify the main feed: ${error.message}. Check the connection and try again.`);
    process.exitCode = 1;
  }
}
