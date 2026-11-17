import { execFileSync } from "node:child_process";

export function builtOn(cwd?: string): string {
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
    return git("status", "--porcelain", "--untracked-files=no") ? `${head}-dirty` : head;
  } catch {
    return "";
  }
}
