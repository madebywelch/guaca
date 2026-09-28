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
/**
 * How this client and its host differ, by release version, then by build.
 *
 * A desktop and its host are updated separately, so either can be ahead. Only
 * stable versions are ordered; a development build or a legacy host with no
 * version is `unknown`, never assumed current. `otherBuild` is one version and
 * two commits: a source build carries the version of the last release, so a
 * version alone reads "same" while one side is missing commands the other has.
 */
export type Skew = "same" | "otherBuild" | "clientBehind" | "hostBehind" | "unknown";
export function skew(client: { version: string; commit: string }, health: Health | null): Skew {
  const order = health?.version ? compareVersions(client.version, health.version) : null;
  if (!health || order === null) return "unknown";
  if (order === 0) return otherBuild(client.commit, health.build) ? "otherBuild" : "same";
  return order < 0 ? "clientBehind" : "hostBehind";
}

/** Whether a published release is newer than this client's own version. */
export function clientUpdate(client: string, status: ReleaseStatus | null): boolean {
  return !!status?.latest && compareVersions(client, status.latest.version) === -1;
}

/** A browser page is the host's own bundle, so a different commit is a stale page. */
export function pageStale(desktop: boolean, commit: string, health: Health | null): boolean {
  return (
    !desktop && !!health?.build && !sameBuild(commit, health.build) && !commit.endsWith("-dirty")
  );
}

export interface UpdateFacts {
  desktop: boolean;
  client: { version: string; commit: string };
  health: Health | null;
  release: ReleaseStatus | null;
  /** The image a host this desktop manages would be replaced with, when it differs. */
  localUpdate: string | null;
}
export interface UpdateNotice {
  /** What a dismissal hides: this message about these versions, and no later one. */
  key: string;
  text: string;
}

/**
 * The one line worth drawing above the conversation, most urgent first.
 *
 * A stale page comes first because every other fact on it is stale too. A
 * skew between a desktop and its host comes next, because it is the one that
 * breaks commands; a newer published release is only news.
 */
export function updateNotice(facts: UpdateFacts): UpdateNotice | null {
  const { desktop, client, health, release, localUpdate } = facts;
  if (!health) return null;
  if (pageStale(desktop, client.commit, health))
    return {
      key: `page:${health.build}`,
      text: "This page has an older Guaca build. Reload when you are ready.",
    };
  const latest = release?.latest?.version;
  const appBehindRelease = desktop && clientUpdate(client.version, release);
  if (desktop) {
    const drift = skew(client, health);
    if (drift === "hostBehind" || drift === "clientBehind")
      return {
        key: `${drift}:${health.version}:${client.version}`,
        text: `This host runs Guaca ${health.version} and this app is ${client.version}. ${
          drift === "hostBehind" ? "Update the host to match." : "Update this app to match."
        }`,
      };
    if (drift === "otherBuild")
      return {
        key: `otherBuild:${health.build}:${client.commit}`,
        text: `This app and its host are different builds of Guaca ${health.version}, so features one has may be missing from the other. Run both from the same build.`,
      };
  }
  if (available(health, release))
    return {
      key: `host:${latest}`,
      text: appBehindRelease
        ? `Guaca ${latest} is available for this app and its host.`
        : `Host update available: Guaca ${latest}.`,
    };
  if (localUpdate) return { key: `local:${localUpdate}`, text: "Host update available." };
  if (appBehindRelease)
    return { key: `app:${latest}`, text: `Guaca ${latest} is available for this app.` };
  return null;
}

export function sameBuild(a: string, b: string): boolean {
  return (
    /^[a-f0-9]{7,40}$/.test(a) && /^[a-f0-9]{7,40}$/.test(b) && (a.startsWith(b) || b.startsWith(a))
  );
}

const BUILD = /^[a-f0-9]{7,40}(-dirty)?$/;

/**
 * Two builds known to be different code. Both have to be commits: an empty
 * build was made without a repository, and nothing is known about it. A
 * `-dirty` build matches only the identical string, which is the most either
 * side can say about a tree with uncommitted edits. `ipc::different_builds`
 * is the host's copy of this rule.
 */
function otherBuild(a: string, b: string): boolean {
  return BUILD.test(a) && BUILD.test(b) && a !== b && !sameBuild(a, b);
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
