import type { HostOperation } from "./host";

export interface Progress {
  steps: readonly string[];
  done: number;
  now: number;
  label: string;
}

export const HOST_STEPS = [
  "Downloading update",
  "Stopping host",
  "Backing up data",
  "Starting updated host",
  "Verifying host",
] as const;

export interface Target {
  image?: string;
  version?: string;
  attemptId?: string;
}

export function hostProgress(
  op: HostOperation | null,
  target: Target,
  unreachable: boolean,
): Progress {
  const steps = HOST_STEPS;
  const ours =
    !!op &&
    (target.attemptId
      ? (op as HostOperation & { attemptId?: string }).attemptId === target.attemptId
      : target.image
        ? op.targetImage === target.image
        : op.targetVersion === target.version);
  if (ours && op?.stage === "Restoring previous version")
    return { steps, done: 0, now: steps.length, label: op.stage };
  if (ours && op.stage === "Host updated")
    return { steps, done: steps.length, now: 0, label: op.stage };
  const at =
    op && (!target.attemptId || ours) ? steps.indexOf(op.stage as (typeof steps)[number]) : -1;
  if (unreachable) {
    const from = Math.max(0, at);
    return {
      steps,
      done: from,
      now: 0,
      label: at < 0 ? "No update step reported yet" : `Last reported: ${steps[at]}`,
    };
  }
  if (at === -1) return { steps, done: 0, now: 1, label: "Starting update" };
  return { steps, done: at, now: 1, label: steps[at]! };
}

export const REBUILD_STEPS = [
  "Checking what this needs",
  "Getting the latest",
  "Building",
  "Building the local host",
  "Signing",
  "Quitting the running Guaca",
] as const;

export function rebuildProgress(stage: string | null): Progress {
  const steps = REBUILD_STEPS;
  if (!stage) return { steps, done: 0, now: 1, label: "Starting the rebuild" };
  const at = steps.indexOf(stage as (typeof steps)[number]);
  if (at === -1) return { steps, done: 0, now: steps.length, label: stage };
  return { steps, done: at, now: 1, label: stage };
}
