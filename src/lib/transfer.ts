import { desktop, invokeLocal } from "./transport";
import type { GroupId } from "./types";
export interface Reconnect {
  kind: string;
  name: string;
  details: Record<string, unknown>;
  agents: string[];
}
export interface GroupArchive {
  format: "guaca-group";
  version: 1;
  tables: Record<string, Record<string, unknown>[]>;
  memories: Record<string, string>;
  files: Record<string, string>;
  reconnect: Reconnect[];
}
export const legacyGroups = () => invokeLocal<{ id: GroupId; name: string }[]>("legacy_groups");
export const exportLegacyGroup = (id: GroupId) =>
  invokeLocal<GroupArchive>("export_legacy_group", { id });
export async function saveGroup(archive: GroupArchive): Promise<string> {
  if (desktop) return invokeLocal<string>("save_group_export", { archive });
  const url = URL.createObjectURL(
    new Blob([JSON.stringify(archive)], { type: "application/json" }),
  );
  const a = document.createElement("a");
  a.href = url;
  a.download = "Guaca-group.guaca.json";
  document.body.append(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
  return "Group export downloaded.";
}

export function parseGroupFile(text: string): GroupArchive {
  const value: unknown = JSON.parse(text);
  const record = (v: unknown): v is Record<string, unknown> =>
    !!v && typeof v === "object" && !Array.isArray(v);
  if (
    !record(value) ||
    value.format !== "guaca-group" ||
    value.version !== 1 ||
    !record(value.tables) ||
    !Array.isArray(value.tables.groups) ||
    value.tables.groups.length !== 1 ||
    !record(value.tables.groups[0]) ||
    typeof value.tables.groups[0].name !== "string" ||
    !Object.values(value.tables).every((rows) => Array.isArray(rows) && rows.every(record)) ||
    !record(value.memories) ||
    !Object.values(value.memories).every((v) => typeof v === "string") ||
    !record(value.files) ||
    !Object.values(value.files).every((v) => typeof v === "string") ||
    !Array.isArray(value.reconnect) ||
    !value.reconnect.every(
      (item) =>
        record(item) &&
        typeof item.kind === "string" &&
        typeof item.name === "string" &&
        record(item.details) &&
        Array.isArray(item.agents) &&
        item.agents.every((a) => typeof a === "string"),
    )
  ) {
    throw new Error("This is not a supported Guaca group file.");
  }
  return value as unknown as GroupArchive;
}
