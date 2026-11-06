import { VERSION } from "./build";
import type { Channel, Health, Skew } from "./releases";
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

/** Which host a window is showing, in the three ways a person can tell. */
export type Place = "thisMac" | "remote" | "browser";

/**
 * Which host this window shows.
 *
 * Read at the call rather than once on import, because the answer is the
 * chooser's and the page is reloaded when the chooser changes it.
 *
 * A window with nothing attached is the app's own runtime, which is on this
 * Mac. An attached one is a box only when the chooser says remote: the desktop
 * reaches a container on this Mac and a host already running on this Mac by
 * the same kind of address, so the stored mode is the only thing that tells
 * them from a server somebody rented. A browser is told neither and would be
 * guessing, which is the same reason `unreachableHint` says `host` for one.
 */
export function hostPlace(): Place {
  if (!desktop) return "browser";
  if (!attached()) return "thisMac";
  return hostMode() === "remote" ? "remote" : "thisMac";
}

const PLACES: Record<Place, string> = {
  thisMac: "This Mac",
  remote: "Remote host",
  browser: "This host",
};

/**
 * The host, as a sentence says it.
 *
 * The status bar's label names the window's arrangement ("This Mac", "Remote
 * host", "This host"); a sentence about where the runtime is has to agree with
 * it. "This machine" was written when there was only one arrangement, and it is
 * false of the two where the runtime is not in this window: a browser is on one
 * machine and its host is on another, and Settings is exactly where somebody
 * goes to learn which holds what.
 */
export function hostAt(place: Place): string {
  return place === "thisMac" ? "this Mac" : "your host";
}

/**
 * What the client in front of the operator keeps of the host's, for About.
 *
 * Three arrangements and three answers, because one sentence was true of one of
 * them. A browser holds no token at all now that the session is an HttpOnly
 * cookie the daemon sets; a desktop window running its own runtime has no
 * address to reach and nothing to send; and a desktop window showing a box keeps
 * that box's address and token so a reload finds it again.
 */
export function clientKeeps(facts: { desktop: boolean; attached: boolean }): string {
  if (!facts.desktop) {
    return "This page keeps your display preferences. Its session is a cookie the host sets, which this page cannot read, and the host's token is not stored here.";
  }
  if (facts.attached) {
    return "This app keeps the host's address, its token, and your display preferences.";
  }
  return "This app keeps your display preferences. Its runtime is its own, so it has no address and no token to keep.";
}

export interface HostLabel {
  /** What the bar says: the place, and the drift when there is one. */
  text: string;
  /** The same fact in a sentence with the versions, for a hover. */
  title: string;
  /** Whether this window and the host are on different builds, which is the
   *  one fact on the bar worth a color. */
  drift: boolean;
}

/**
 * The status bar's left item: which host, and whether its build agrees with
 * this window's.
 *
 * It read `Host 0.2.0`, which is the least useful half of what the bar knows.
 * A version is what an operator quotes in a bug report, not something they act
 * on, and it is one click away in Workspace settings. What the bar is for is
 * which machine is doing the work, and whether the two halves are one build,
 * because a skew between them is the fact on that bar that breaks commands.
 *
 * The drift is said in words as well as marked in color. A color alone is a
 * difference only an operator who already knows the rule can read, and the bar
 * said a version for either build once already.
 */
export function hostLabel(facts: {
  place: Place;
  drift: Skew;
  host: Health | null;
  /** This window's own version, named when the two differ. */
  appVersion?: string;
}): HostLabel {
  const { place, drift, host } = facts;
  const app = facts.appVersion ?? VERSION;
  const where = PLACES[place];
  const version = host?.version;
  const said =
    drift === "otherBuild"
      ? " · other build"
      : drift === "clientBehind"
        ? " · app older"
        : drift === "hostBehind"
          ? " · host older"
          : "";
  let title: string;
  if (!version) {
    title = "Which host this window shows. Open Workspace settings for its updates.";
  } else if (drift === "same") {
    title = `${where} runs Guaca ${version}, the same build as this app.`;
  } else if (drift === "otherBuild") {
    title = `${where} runs Guaca ${version}, a different build from this app. Open Workspace settings to see which.`;
  } else if (drift === "clientBehind") {
    title = `${where} runs Guaca ${version} and this app is ${app}. Update this app.`;
  } else if (drift === "hostBehind") {
    title = `${where} runs Guaca ${version} and this app is ${app}. Open Workspace settings to update the host.`;
  } else {
    title = `${where} runs Guaca ${version}. Open Workspace settings for its updates.`;
  }
  return { text: `${where}${said}`, title, drift: said !== "" };
}
