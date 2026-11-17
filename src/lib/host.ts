import { VERSION } from "./build";
import type { Channel, Health, Skew } from "./releases";
import { attached, desktop, invokeLocal, type Remote, savedHostMode } from "./transport";

export interface HostOperation {
  stage: string;
  backup: string | null;
  previousImage: string;
  targetImage: string;
  targetVersion?: string;
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

export type UpdateFailure = "refused" | "unanswered";

export interface Manager {
  channel?: Channel;
  updating: boolean;
  running: { image: string; version: string | null } | null;
  operation: HostOperation | null;
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
export interface AppSource {
  checkout: string | null;
  unavailable: string | null;
  branch: string | null;
  upstream: string | null;
  running: boolean;
  stage: string | null;
  failure: string | null;
  log: string;
}
export const thisApp = {
  source: () => invokeLocal<AppSource>("app_source"),
  rebuild: () => invokeLocal<void>("rebuild_app"),
};
export function hostMode() {
  return savedHostMode();
}

export function unreachableHint(): string {
  if (!desktop) return "Check that the host is up, and that this address still answers from here.";
  if (hostMode() === "existing") return "Check that the host is still running on this Mac.";
  if (hostMode() === "remote" && attached())
    return "Check that the box is up, and that its address or tunnel still reaches it.";
  return "Check that Docker is running: Guaca runs in a container on this Mac.";
}

export type Place = "thisMac" | "remote" | "browser";

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

export function hostAt(place: Place): string {
  return place === "thisMac" ? "this Mac" : "your host";
}

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
  text: string;
  title: string;
  drift: boolean;
}

export function hostLabel(facts: {
  place: Place;
  drift: Skew;
  host: Health | null;
  appVersion?: string;
  lastHeard?: number | null;
}): HostLabel {
  const { place, drift, host } = facts;
  const app = facts.appVersion ?? VERSION;
  const where = PLACES[place];
  const version = host?.version;
  if (facts.lastHeard != null) {
    const clock = new Date(facts.lastHeard).toLocaleTimeString([], {
      hour: "numeric",
      minute: "2-digit",
    });
    return {
      text: `${where} · last heard ${clock}`,
      title: `${where} has not answered since ${clock}, so this page is not live. It catches up when the host does.`,
      drift: false,
    };
  }
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
    title = "Which host this window shows. Open Host settings for its updates.";
  } else if (drift === "same") {
    title = `${where} runs Guaca ${version}, the same build as this app.`;
  } else if (drift === "otherBuild") {
    title = `${where} runs Guaca ${version}, a different build from this app. Open Host settings to see which.`;
  } else if (drift === "clientBehind") {
    title = `${where} runs Guaca ${version} and this app is ${app}. Update this app.`;
  } else if (drift === "hostBehind") {
    title = `${where} runs Guaca ${version} and this app is ${app}. Open Host settings to update the host.`;
  } else {
    title = `${where} runs Guaca ${version}. Open Host settings for its updates.`;
  }
  return { text: `${where}${said}`, title, drift: said !== "" };
}
