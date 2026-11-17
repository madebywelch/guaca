export function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

const ENTRY_KEYS = ["name", "agent", "path", "file"];

const FILE_KEYS = ["files", "attachments", "paths", "path", "file"];

function firstPresent(record: Record<string, unknown>, keys: string[]): unknown {
  const key = keys.find((candidate) => Object.hasOwn(record, candidate));
  return key === undefined ? undefined : record[key];
}

function entryName(entry: unknown): string {
  if (typeof entry === "string") return entry;
  const found = firstPresent(asRecord(entry), ENTRY_KEYS);
  return typeof found === "string" ? found : "";
}

function nameList(raw: unknown): string[] {
  const names =
    typeof raw === "string" ? raw.split(",") : Array.isArray(raw) ? raw.map(entryName) : [];
  const seen = new Set<string>();
  return names
    .map((name) => name.trim())
    .filter((name) => {
      const key = name.toLowerCase();
      if (!name || seen.has(key)) return false;
      seen.add(key);
      return true;
    });
}

export function sendRecipients(args: unknown): string[] {
  const record = asRecord(args);
  const to = nameList(record.to);
  if (to.length > 0) return to;
  const single = typeof record.agent === "string" ? record.agent.trim() : "";
  return single ? [single] : [];
}

export function attachedFiles(args: unknown): string[] {
  return nameList(firstPresent(asRecord(args), FILE_KEYS));
}

export function attachedNames(args: unknown): string[] {
  return attachedFiles(args).map((path) => path.split(/[/\\]/).filter(Boolean).pop() ?? path);
}

export function sendBody(args: unknown): string {
  const record = asRecord(args);
  const text = record.text ?? record.message;
  return typeof text === "string" ? text : "";
}
