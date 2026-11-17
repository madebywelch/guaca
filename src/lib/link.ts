export type Link = "up" | "down";

const RESTART_QUIET_MS = 180_000;

let quietUntil = 0;

export function expectRestart(): void {
  quietUntil = Date.now() + RESTART_QUIET_MS;
}

export function restarting(now = Date.now()): boolean {
  return now < quietUntil;
}

export function restartQuietRemaining(now = Date.now()): number {
  return Math.max(0, quietUntil - now);
}

export function forgetRestart(): void {
  quietUntil = 0;
}
