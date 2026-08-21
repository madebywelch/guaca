/**
 * Reads tool-call arguments back out of a stored message.
 *
 * The arguments were produced by a model, so they can be any shape at all,
 * including shapes the schema forbade. Parsing is deliberately forgiving: the
 * runtime already decided what to do with the call, and this only has to
 * recover enough to show the operator what was sent.
 */

/**
 * A tool call's arguments, read as an object.
 *
 * Exported because the trail reads the same arguments back from the same
 * messages: an argument list that is not an object is a call with no arguments
 * worth showing, and both readers have to agree on that.
 */
export function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

/**
 * A list of names out of whatever the model sent, given the keys an object
 * entry might hide one under.
 *
 * The shapes are the ones `normalize_list` in `tools.rs` accepts, because both
 * sides are reading the same call: a bare string, a comma-separated string, an
 * array of strings, an array of objects.
 */
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

/** Recipient names, from any of the shapes models actually emit. */
export function sendRecipients(args: unknown): string[] {
  const record = asRecord(args);
  return nameList(record.to ?? record.agent, ["name", "agent"]);
}

/**
 * The files a call named, as a person would say them.
 *
 * Leaf names only. What a model passes is a path on its own machine, and the
 * directory it kept the file in is the one fact about it the operator has no
 * use for: they cannot reach that disk, which is the whole reason the file was
 * attached instead of named.
 */
export function attachedNames(args: unknown): string[] {
  const record = asRecord(args);
  const raw = record.files ?? record.attachments ?? record.paths ?? record.path ?? record.file;
  return nameList(raw, ["name", "path", "file"]).map(
    (path) => path.split(/[/\\]/).filter(Boolean).pop() ?? path,
  );
}

/** The message body that was sent, or an empty string. */
export function sendBody(args: unknown): string {
  const record = asRecord(args);
  const text = record.text ?? record.message;
  return typeof text === "string" ? text : "";
}
