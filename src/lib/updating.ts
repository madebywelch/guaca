import type { HostOperation } from "./host";

/**
 * How far an update has got, in the steps whatever runs it reports.
 *
 * Nothing that runs an update knows how long a step has left: a pull is
 * Docker's, a backup is a copy of a volume nobody measured, a build is cargo's.
 * So progress is counted in steps and never in time. A step is drawn finished
 * only once it is reported finished, and the one underway moves in place
 * rather than filling toward a guess.
 */
export interface Progress {
  /** Every step, in the words the operator reads. */
  steps: readonly string[];
  /** How many of them are finished. */
  done: number;
  /** How many after those it may be in: one when it says which, more when it cannot. */
  now: number;
  /** What is happening. */
  label: string;
}

/**
 * A host update's steps, as `host::Stage` names them in the journal. The same
 * sequence runs on a desktop and on a box. The test beside this reads the
 * names from `host.rs`.
 */
export const HOST_STEPS = [
  "Downloading update",
  "Stopping host",
  "Backing up workspace",
  "Starting updated host",
  "Verifying host",
] as const;

/** What an update installs, which is how its journal entry is told from the last one's. */
export interface Target {
  image?: string;
  version?: string;
}

export function hostProgress(
  op: HostOperation | null,
  target: Target,
  /** A box's host not answering, which the box's updater can only be read through. */
  unreachable: boolean,
): Progress {
  const steps = HOST_STEPS;
  if (op?.stage === "Restoring previous version")
    return { steps, done: 0, now: steps.length, label: op.stage };
  const ours =
    !!op && (target.image ? op.targetImage === target.image : op.targetVersion === target.version);
  if (ours && op.stage === "Host updated")
    return { steps, done: steps.length, now: 0, label: op.stage };
  // A stage still underway can only be this update's. A finished one is the
  // last update's until the manager records this one, which it does after
  // checking the target and before anything is downloaded.
  const at = op ? steps.indexOf(op.stage as (typeof steps)[number]) : -1;
  if (unreachable) {
    // A box's host that stops answering was stopped by the update, and the
    // updater behind it says nothing until a host is answering again: it is
    // stopping, backing up or starting, and which of the three is not
    // something a client can know.
    const from = Math.max(at, steps.indexOf("Stopping host"));
    const to = Math.max(from, steps.indexOf("Starting updated host"));
    return { steps, done: from, now: to - from + 1, label: "Waiting for the host to answer" };
  }
  if (at === -1) return { steps, done: 0, now: 1, label: "Starting update" };
  return { steps, done: at, now: 1, label: steps[at]! };
}

/**
 * What `scripts/install.sh` announces, in order, up to quitting this app. It
 * announces more after that, which this app is no longer running to read. The
 * test beside this reads them from the script.
 */
export const REBUILD_STEPS = [
  "Checking what this needs",
  "Getting the latest",
  "Building",
  "Building the local host",
  "Signing",
  "Quitting the running Guaca",
] as const;

/** A rebuild, from the step the script last announced in its log. */
export function rebuildProgress(stage: string | null): Progress {
  const steps = REBUILD_STEPS;
  if (!stage) return { steps, done: 0, now: 1, label: "Starting the rebuild" };
  const at = steps.indexOf(stage as (typeof steps)[number]);
  // The script is the one the rebuild pulled, which can be newer than this
  // app and name a step it does not know. Said, but not placed.
  if (at === -1) return { steps, done: 0, now: steps.length, label: stage };
  // A step that did not run (no Docker, no local host to build) is behind
  // the one announced after it, and drawn finished like the rest.
  return { steps, done: at, now: 1, label: stage };
}
