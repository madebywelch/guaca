import { invokeLocal, type Remote } from "./transport";

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
const MODE = "guaca.workspace.hostMode";
type HostMode = "local" | "existing" | "remote";
export function hostMode(): HostMode {
  const saved = localStorage.getItem(MODE);
  return saved === "local" || saved === "existing" ? saved : "remote";
}
export function rememberMode(mode: HostMode) {
  localStorage.setItem(MODE, mode);
}
