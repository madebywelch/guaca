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

/** Recipient names, from any of the shapes models actually emit. */
export function sendRecipients(args: unknown): string[] {
  const record = asRecord(args);
  const raw = record.to ?? record.agent;

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
        const nested = asRecord(entry);
        const name = nested.name ?? nested.agent;
        return typeof name === "string" ? name.trim() : "";
      })
      .filter(Boolean);
  }
  return [];
}

/** The message body that was sent, or an empty string. */
export function sendBody(args: unknown): string {
  const record = asRecord(args);
  const text = record.text ?? record.message;
  return typeof text === "string" ? text : "";
}
