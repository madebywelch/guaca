/**
 * What this build is, which is the commit it was made from.
 *
 * `__COMMIT__` is replaced by `vite.config.ts` when the bundle is built. There
 * is nothing to ask at runtime: the app ships without the repository it came
 * from, and the version in `package.json` and `tauri.conf.json` is a placeholder
 * that has never moved. Empty is a build made outside a repository (a source
 * tarball, or a CI image that copied no `.git`), which is a thing to say rather
 * than a failure to report.
 */

declare const __COMMIT__: string;

/** The commit this build was made from, suffixed `-dirty` where the tree had
 *  uncommitted edits on top of it, and empty where there was none to read. */
export const COMMIT: string = __COMMIT__;

/** The line About draws. A dash rather than a blank, for the reason every other
 *  unreadable fact in this app gets one: an empty space reads as a bug in the
 *  pane, and there is nothing here for an operator to act on either way. */
export function buildLabel(commit: string = COMMIT): string {
  return `Version ${commit || "—"}`;
}
