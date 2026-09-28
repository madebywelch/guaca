import { execFileSync } from "node:child_process";

/**
 * The commit this bundle is being built from.
 *
 * The number in `package.json` has not moved since the first commit and is not
 * going to: what ships here is a commit rather than a release, so a version
 * read off that file tells an operator nothing and tells a bug report less.
 * Read here because the built app has no repository to ask, and at build time
 * because that is when the answer stops changing: a dev server keeps the commit
 * it started on, which is the commit the bundle behind it was built from.
 */
export function builtOn(cwd?: string): string {
  // An image build has no `.git` in its context and is told the commit
  // instead, so the page's About and the daemon's /health say the same thing.
  const told = process.env.GUACA_COMMIT?.trim();
  if (told) return told;
  const git = (...args: string[]) =>
    execFileSync("git", args, {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  try {
    const head = git("rev-parse", "--short=7", "HEAD");
    // A tree with edits on top of that commit did not produce this build, and
    // an unqualified hash says it did. The difference is somebody checking out
    // that commit and hunting for a defect that was never in it. A file git
    // does not track is not such an edit: nothing is built from it unless a
    // tracked file names it, and naming it is one. Counted, a stray note in
    // the checkout made a build of main read as a different build from the
    // same commit on a box. `scripts/install.sh` decides "clean" the same way.
    return git("status", "--porcelain", "--untracked-files=no") ? `${head}-dirty` : head;
  } catch {
    // No git, no repository, or a source tarball. About draws a dash.
    return "";
  }
}
