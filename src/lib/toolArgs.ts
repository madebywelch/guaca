export function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

function nameList(raw: unknown, nested: string[]): string[] {
  if (typeof raw === "string") {
    return raw
      .split(",")
      .map((name) => name.trim())
      .filter(Boolean);
  }
  if (Array.isArray(raw)) {
    return raw
      .map((entry) => {
        if (typeof entry === "string") return entry.trim();
        const record = asRecord(entry);
        const found = nested.map((key) => record[key]).find((v) => typeof v === "string");
        return typeof found === "string" ? found.trim() : "";
      })
      .filter(Boolean);
  }
  return [];
}

export function sendRecipients(args: unknown): string[] {
  const record = asRecord(args);
  return nameList(record.to ?? record.agent, ["name", "agent"]);
}

export function attachedNames(args: unknown): string[] {
  const record = asRecord(args);
  const raw = record.files ?? record.attachments ?? record.paths ?? record.path ?? record.file;
  return nameList(raw, ["name", "path", "file"]).map(
    (path) => path.split(/[/\\]/).filter(Boolean).pop() ?? path,
  );
}

export function sendBody(args: unknown): string {
  const record = asRecord(args);
  const text = record.text ?? record.message;
  return typeof text === "string" ? text : "";
}
