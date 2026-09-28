import { whenLabel } from "./time";
import type {
  Artifact,
  ArtifactActor,
  ArtifactChange,
  ArtifactEntry,
  ArtifactRead,
  ArtifactSource,
} from "./types";

/**
 * What the Artifacts dialog says about a kept page, in the operator's words.
 *
 * No DOM, so every sentence the dialog draws is one a test can read. The Rust
 * side has its own rendering of the same log for agents, in their words
 * (`domain::artifact::log_lines`): an agent is told "you", and so is the
 * operator, and those are two different readers.
 */

/** Whether a change made a version, and so has a page to open. */
export function makesVersion(change: ArtifactChange): boolean {
  return change === "created" || change === "edited" || change === "restored";
}

/** Who did something, for the operator reading about it. */
export function actorLabel(actor: ArtifactActor): string {
  return actor.kind === "operator" ? "You" : actor.name;
}

/**
 * Who answers for it. An owner who left is still named, and said to have left:
 * ownership only moves when somebody decides to move it, so the list has to
 * say that nobody has yet.
 */
export function ownerLabel(owner: Artifact["owner"]): string {
  if (owner === null) return "Nobody";
  return owner.gone ? `${owner.name}, left the crew` : owner.name;
}

/**
 * Everything about an artifact that is not the page, as one quiet line.
 *
 * One line and not columns, because none of it is what the operator opened the
 * dialog for: they came for the page. Whose crew it is leads the line only
 * while every crew is listed, since inside one it would say the same crew on
 * every row.
 */
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

/** One row of the history as the dialog draws it. */
export interface LogRow {
  key: string;
  at: number;
  version: number;
  who: string;
  what: string;
  note: string;
  /** A row that made a version can be opened and put back; the others cannot. */
  opens: boolean;
}

/**
 * The log, newest first, with ownership changes said in full.
 *
 * No row records who held the page before a take-over. It is the owner on the
 * row before, which is why this walks the whole log in order rather than
 * rendering rows one at a time, and only then turns it around.
 */
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

/**
 * What a page's `guaca.data()` resolves to: one entry per source, by name.
 *
 * Keyed by name because that is what the agent wrote the page against, and
 * every source is present, read or refused, so a page can always say which of
 * its numbers are missing rather than failing on one it expected.
 */
export function pageData(
  reads: ArtifactRead[],
): Record<string, { data: unknown; text: string | null; error: string | null }> {
  const out: Record<string, { data: unknown; text: string | null; error: string | null }> = {};
  for (const read of reads) {
    out[read.name] = { data: read.data ?? null, text: read.text, error: read.error };
  }
  return out;
}

/**
 * One source as the operator reads it before allowing it: which connector,
 * which of its tools, and exactly what it will be sent. The arguments are the
 * part that matters, because they are what the page can never change.
 */
export function sourceLine(source: ArtifactSource): string {
  const [server, ...rest] = source.tool.split("__");
  const tool = rest.join("__");
  const args =
    source.arguments && typeof source.arguments === "object" && Object.keys(source.arguments).length
      ? ` ${JSON.stringify(source.arguments)}`
      : "";
  return `${server} · ${tool}${args}`;
}

/**
 * The message a kept page's send becomes, sent to the page's owner as the
 * operator's, because it was their click.
 *
 * Guaca's sentence around the page's JSON, never the page's own words, for the
 * reason `answerMessage` gives: nothing a page wrote may arrive as an
 * instruction in the operator's voice. It names the page and its id, so the
 * owner can `update` it without looking it up, and says the value is the
 * page's, so it is read as data from a control rather than as a request.
 */
export function sentMessage(artifact: Artifact, json: string): string {
  const longest = Math.max(0, ...[...json.matchAll(/`+/g)].map((run) => run[0].length));
  const rail = "`".repeat(Math.max(3, longest + 1));
  return (
    `I pressed a control on "${artifact.title}" (artifact ${artifact.id}, version ${artifact.version}). ` +
    `The page sent this:\n\n${rail}json\n${json}\n${rail}`
  );
}
