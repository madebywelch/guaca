import { invokeLocal, type Remote } from "./transport";

export interface DockerStatus {
  state: "missing" | "unavailable" | "ready" | "running" | "stopped";
  message: string;
  updateAvailable: boolean;
  updating?: boolean;
  origin?: string | null;
  targetImage?: string;
  targetVersion?: string;
  operation?: {
    stage: string;
    backup: string | null;
    previousImage: string;
    targetImage: string;
    error: string | null;
  } | null;
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
