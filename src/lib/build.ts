/**
 * What this build is: the version it ships as, and the commit it was made from.
 *
 * `__VERSION__` and `__COMMIT__` are replaced by `vite.config.ts` when the
 * bundle is built. There is nothing to ask at runtime: the app ships without
 * the repository it came from.
 *
 * The version is the one a release is published under. `package.json`,
 * `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml` move together for a
 * release, and the daemon answers the same number on `/health`, so it is what an
 * operator quotes in a bug report. The commit is the half the number cannot
 * say: `main` and a release can both read 0.2.0 and be a week apart.
 *
 * Empty is a build made outside a repository (a source tarball, or a CI image
 * that copied no `.git`), which is a thing to say rather than a failure to
 * report.
 */

declare const __COMMIT__: string;
declare const __VERSION__: string;
export const VERSION: string = __VERSION__;

/** The commit this build was made from, suffixed `-dirty` where the tree had
 *  uncommitted edits on top of it, and empty where there was none to read. */
export const COMMIT: string = __COMMIT__;

/**
 * The line About draws.
 *
 * Version first, because that is the fact a person carries to a bug report.
 * The commit follows in parentheses the way a Mac app writes a build number,
 * and stands alone only where there is no version to put in front of it. A dash
 * rather than a blank when there is neither, for the reason every other
 * unreadable fact in this app gets one: an empty space reads as a bug in the
 * pane, and there is nothing here for an operator to act on either way.
 */
export function buildLabel(version: string = VERSION, commit: string = COMMIT): string {
  if (!version) return `Version ${commit || "—"}`;
  return commit ? `Version ${version} (${commit})` : `Version ${version}`;
}
