import { invokeLocal, type Remote } from "./transport";

export interface DockerStatus {
  state: "missing" | "unavailable" | "ready" | "running" | "stopped";
  message: string;
  updateAvailable: boolean;
}
export const localHost = {
  status: () => invokeLocal<DockerStatus>("local_host_status"),
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
