import protocol from "../../release-protocol.json";

export { protocol };
export const RELEASES = "https://github.com/madebywelch/guaca/releases";
export const INSTRUCTIONS =
  "https://github.com/madebywelch/guaca/blob/main/docs/HOSTING.md#updating-a-self-hosted-backend";

export interface Health {
  service: "guacad";
  build: string;
  version?: string;
  release?: boolean;
  apiGeneration?: number;
}
export interface Release {
  schema: number;
  version: string;
  commit: string;
  image: string;
  apiGeneration: number;
  clientMinimum: number;
  clientMaximum: number;
  notes: string;
}
export interface ReleaseStatus {
  automatic: boolean;
  checkedAt: string | null;
  latest: Release | null;
  error: string | null;
}
export type Compatibility = "compatible" | "hostOld" | "clientOld" | "unknown";
export function compatibility(health: Health): Compatibility {
  const generation = health.apiGeneration;
  if (!Number.isSafeInteger(generation) || !generation || generation < 0) return "unknown";
  if (generation < protocol.minimum) return "hostOld";
  if (generation > protocol.maximum) return "clientOld";
  return "compatible";
}

/** Stable published versions only. Never order commits or custom builds. */
export function compareVersions(left: string, right: string): number | null {
  const parse = (s: string) =>
    /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(s) ? s.split(".").map(Number) : null;
  const a = parse(left),
    b = parse(right);
  if (!a || !b || [...a, ...b].some((v) => !Number.isSafeInteger(v))) return null;
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i]! < b[i]! ? -1 : 1;
  return 0;
}
export function available(health: Health | null, status: ReleaseStatus | null): boolean {
  return !!(
    health?.release &&
    health.version &&
    status?.latest &&
    compareVersions(health.version, status.latest.version) === -1
  );
}
export function sameBuild(a: string, b: string): boolean {
  return (
    /^[a-f0-9]{7,40}$/.test(a) && /^[a-f0-9]{7,40}$/.test(b) && (a.startsWith(b) || b.startsWith(a))
  );
}

export function parseHealth(value: unknown): Health {
  if (!value || typeof value !== "object" || !("service" in value) || value.service !== "guacad")
    throw new Error(
      "This address did not answer as a Guaca host. Check the connection in Workspace settings.",
    );
  const data = value as Record<string, unknown>;
  return {
    service: "guacad",
    build: typeof data.build === "string" ? data.build : "",
    version: typeof data.version === "string" ? data.version : undefined,
    release: data.release === true,
    apiGeneration: typeof data.apiGeneration === "number" ? data.apiGeneration : undefined,
  };
}

export function parseReleaseStatus(value: unknown): ReleaseStatus {
  if (!value || typeof value !== "object")
    throw new Error("The host returned unreadable update information.");
  const data = value as ReleaseStatus;
  if (
    typeof data.automatic !== "boolean" ||
    !(data.checkedAt === null || typeof data.checkedAt === "string") ||
    !(data.error === null || typeof data.error === "string")
  )
    throw new Error("The host returned unreadable update information.");
  const r = data.latest;
  if (
    r !== null &&
    (r?.schema !== 1 ||
      typeof r.version !== "string" ||
      compareVersions(r.version, r.version) !== 0 ||
      !/^[a-f0-9]{40}$/.test(r.commit) ||
      !/^ghcr\.io\/madebywelch\/guaca\/guacad@sha256:[a-f0-9]{64}$/.test(r.image) ||
      r.notes !== `${RELEASES}/tag/v${r.version}` ||
      !Number.isSafeInteger(r.apiGeneration) ||
      r.apiGeneration < 1 ||
      !Number.isSafeInteger(r.clientMinimum) ||
      !Number.isSafeInteger(r.clientMaximum) ||
      r.clientMinimum < 1 ||
      r.clientMinimum > r.apiGeneration ||
      r.clientMaximum < r.apiGeneration)
  )
    throw new Error("The release metadata could not be verified. Try again later.");
  return data;
}
