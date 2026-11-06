import type { Channel } from "./releases";
import { attached, desktop, invokeLocal, type Remote } from "./transport";

/** One host update, as the manager that ran it recorded it. */
export interface HostOperation {
  stage: string;
  backup: string | null;
  previousImage: string;
  targetImage: string;
  targetVersion?: string;
  /** Where the workspace a failed update left was copied before the backup went back. */
  preserved?: string | null;
  error: string | null;
}
export interface DockerStatus {
  state: "missing" | "unavailable" | "ready" | "running" | "stopped";
  message: string;
  updateAvailable: boolean;
  updating?: boolean;
  origin?: string | null;
  targetImage?: string;
  targetVersion?: string;
  operation?: HostOperation | null;
}

/** A box's updater, as the host relays it. `null` is a host with none. */
export interface Manager {
  /** What it installs. Absent from an updater older than channels. */
  channel?: Channel;
  updating: boolean;
  running: { image: string; version: string | null } | null;
  operation: HostOperation | null;
  /** Why the updater could not start the host, until it can. */
  error: string | null;
}
export function parseManager(value: unknown): Manager | null {
  const unreadable = () => new Error("The host returned unreadable update information.");
  if (!value || typeof value !== "object") throw unreadable();
  const data = value as Record<string, unknown>;
  if (data.managed === false) return null;
  const op = data.operation as Record<string, unknown> | null;
  if (
    data.managed !== true ||
    !(data.channel === undefined || data.channel === "release" || data.channel === "main") ||
    typeof data.updating !== "boolean" ||
    !(data.error === null || typeof data.error === "string") ||
    !(op === null || (typeof op === "object" && typeof op.stage === "string"))
  )
    throw unreadable();
  return data as unknown as Manager;
}
export interface ExistingHost {
  name: string;
  label: string;
  origin: string;
}
export const localHost = {
  existing: () => invokeLocal<ExistingHost[]>("local_hosts"),
  connect: (name: string) => invokeLocal<Remote>("connect_local_host", { name }),
  status: () => invokeLocal<DockerStatus>("local_host_status"),
  update: (origin?: string) => invokeLocal<Remote>("local_host_update", { origin }),
  start: () => invokeLocal<Remote>("local_host_start"),
  openDocker: () => invokeLocal<void>("open_docker"),
};
/** The checkout a source build was made from, as the desktop reads it. */
export interface AppSource {
  checkout: string | null;
  /** Why this app cannot rebuild itself, when it cannot. */
  unavailable: string | null;
  branch: string | null;
  /** The commit that branch is at on origin, when it could be read. */
  upstream: string | null;
  running: boolean;
  /** The step `install.sh` last announced, while it runs. */
  stage: string | null;
  /** The end of the log of the rebuild that last failed. */
  failure: string | null;
  log: string;
}
/** This app, rebuilt from its own checkout by `scripts/install.sh`. */
export const thisApp = {
  source: () => invokeLocal<AppSource>("app_source"),
  rebuild: () => invokeLocal<void>("rebuild_app"),
};
const MODE = "guaca.workspace.hostMode";
type HostMode = "local" | "existing" | "remote";
export function hostMode(): HostMode {
  const saved = localStorage.getItem(MODE);
  return saved === "local" || saved === "existing" ? saved : "remote";
}
export function rememberMode(mode: HostMode) {
  localStorage.setItem(MODE, mode);
}

/**
 * What to check when nothing answers, in the words of the workspace this page
 * shows.
 *
 * Three arrangements and three answers, because what is usually wrong differs
 * in each: a container on this Mac, a host running on this Mac outside the app,
 * and a host reached over a network. None of them guesses at what failed — the
 * banner has already said the host did not answer — they say which thing a
 * person can go and look at, which is the half the banner cannot supply.
 *
 * A browser says `host` rather than a box or a container because it cannot tell
 * the two apart, and guessing would send somebody to Docker for a server in
 * another country.
 */
export function unreachableHint(): string {
  if (!desktop) return "Check that the host is up, and that this address still answers from here.";
  if (attached())
    return "Check that the box is up, and that its address or tunnel still reaches it.";
  if (hostMode() === "existing") return "Check that the host is still running on this Mac.";
  return "Check that Docker is running: Guaca runs in a container on this Mac.";
}
