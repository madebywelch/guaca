declare const __COMMIT__: string;
declare const __VERSION__: string;
export const VERSION: string = __VERSION__;

export const COMMIT: string = __COMMIT__;

export function buildLabel(version: string = VERSION, commit: string = COMMIT): string {
  if (!version) return `Version ${commit || "unknown"}`;
  return commit ? `Version ${version} (${commit})` : `Version ${version}`;
}
