import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { builtOn } from "../../build-stamp";

/** A repository with one commit, and what `git` calls that commit. */
function checkout(): { dir: string; head: string } {
  const dir = mkdtempSync(join(tmpdir(), "guaca-stamp-"));
  const git = (...args: string[]) =>
    execFileSync("git", args, { cwd: dir, encoding: "utf8" }).trim();
  git("init", "--quiet");
  writeFileSync(join(dir, "tracked.txt"), "one\n");
  git("add", "tracked.txt");
  git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "--quiet", "-m", "one");
  return { dir, head: git("rev-parse", "--short=7", "HEAD") };
}

describe("the commit a build says it was made from", () => {
  afterEach(() => vi.unstubAllEnvs());

  it("is dirty when a tracked file was edited on top of it", () => {
    vi.stubEnv("GUACA_COMMIT", "");
    const { dir, head } = checkout();
    writeFileSync(join(dir, "tracked.txt"), "two\n");
    expect(builtOn(dir)).toBe(`${head}-dirty`);
  });

  it("is not dirty because of a file nothing builds from", () => {
    vi.stubEnv("GUACA_COMMIT", "");
    const { dir, head } = checkout();
    writeFileSync(join(dir, "google.rs"), "// a note, untracked\n");
    expect(builtOn(dir)).toBe(head);
  });

  it("is what an image build was told, since it has no repository to ask", () => {
    vi.stubEnv("GUACA_COMMIT", "d67dcc560cd250098f5ca8ad18016929b8981b06");
    expect(builtOn(checkout().dir)).toBe("d67dcc560cd250098f5ca8ad18016929b8981b06");
  });
});
