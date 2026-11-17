import { whenLabel } from "./time";
import type {
  Artifact,
  ArtifactActor,
  ArtifactChange,
  ArtifactEntry,
  ArtifactRead,
  ArtifactSource,
} from "./types";

export function makesVersion(change: ArtifactChange): boolean {
  return change === "created" || change === "edited" || change === "restored";
}

export function actorLabel(actor: ArtifactActor): string {
  return actor.kind === "operator" ? "You" : actor.name;
}

export function ownerLabel(owner: Artifact["owner"]): string {
  if (owner === null) return "Nobody";
  return owner.gone ? `${owner.name}, left the group` : owner.name;
}

export function detailsLine(artifact: Artifact, crew: string | null, now: number): string {
  return [
    crew,
    `Owner ${ownerLabel(artifact.owner)}`,
    `v${artifact.version}`,
    whenLabel(artifact.updatedAt, now),
  ]
    .filter(Boolean)
    .join(" · ");
}

export interface LogRow {
  key: string;
  at: number;
  version: number;
  who: string;
  what: string;
  note: string;
  opens: boolean;
}

export function logRows(log: ArtifactEntry[]): LogRow[] {
  const rows: LogRow[] = [];
  let before: string | null = null;
  for (const entry of log) {
    const held = before ?? "nobody";
    const what =
      entry.change === "created"
        ? "created it"
        : entry.change === "edited"
          ? "edited it"
          : entry.change === "restored"
            ? "put back an earlier version"
            : entry.change === "took"
              ? `took it over from ${held}`
              : entry.change === "allowed"
                ? "allowed its reads"
                : `handed it from ${held} to ${entry.owner ?? "nobody"}`;
    rows.push({
      key: String(entry.seq),
      at: entry.at,
      version: entry.version,
      who: actorLabel(entry.by),
      what,
      note: entry.note,
      opens: makesVersion(entry.change),
    });
    before = entry.owner;
  }
  return rows.reverse();
}

export function pageData(
  reads: ArtifactRead[],
): Record<string, { data: unknown; text: string | null; error: string | null }> {
  const out: Record<string, { data: unknown; text: string | null; error: string | null }> = {};
  for (const read of reads) {
    out[read.name] = { data: read.data ?? null, text: read.text, error: read.error };
  }
  return out;
}

export function sourceLine(source: ArtifactSource): string {
  const [server, ...rest] = source.tool.split("__");
  const tool = rest.join("__");
  const args =
    source.arguments && typeof source.arguments === "object" && Object.keys(source.arguments).length
      ? ` ${JSON.stringify(source.arguments)}`
      : "";
  return `${server} · ${tool}${args}`;
}

export function sentMessage(artifact: Artifact, json: string): string {
  const longest = Math.max(0, ...[...json.matchAll(/`+/g)].map((run) => run[0].length));
  const rail = "`".repeat(Math.max(3, longest + 1));
  return (
    `I confirmed a value from "${artifact.title}" (artifact ${artifact.id}, version ${artifact.version}). ` +
    `The page sent this:\n\n${rail}json\n${json}\n${rail}`
  );
}
