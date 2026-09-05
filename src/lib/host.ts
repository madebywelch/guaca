import { invokeLocal, type Remote } from "./transport";

export interface DockerStatus {
  state: "missing" | "unavailable" | "ready" | "running" | "stopped";
  message: string;
  updateAvailable: boolean;
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
  update: () => invokeLocal<Remote>("local_host_update"),
  start: () => invokeLocal<Remote>("local_host_start"),
  openDocker: () => invokeLocal<void>("open_docker"),
};
const MODE = "guaca.workspace.hostMode";
export function hostMode(): "local" | "remote" {
  return localStorage.getItem(MODE) === "local" ? "local" : "remote";
}
export function rememberMode(mode: "local" | "remote") {
  localStorage.setItem(MODE, mode);
}
