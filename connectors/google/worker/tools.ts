/**
 * What an agent can actually do with a grant this origin holds.
 *
 * `connectors.ts` says what a person may authorize, in their words, for a
 * consent screen. This says what a model may call, in its words, for a tool
 * definition. They are deliberately two lists: one is a promise to a person and
 * the other is an API surface, and collapsing them would mean either a consent
 * screen written in JSON Schema or a tool list written in marketing.
 *
 * ## The list is derived from the grant, not from the catalogue
 *
 * A tool is offered only when every scope it needs came back from the provider.
 * That is not defensiveness: a capability can be half-granted, a person can
 * narrow a grant later, and Google's consent screen lets someone untick a box
 * on the way through. A model offered `gmail_send` against a grant that never
 * got `gmail.send` spends a turn discovering a 403, reports the task as
 * impossible, and is right for the wrong reason.
 *
 * ## Why the arguments are narrow
 *
 * Every tool here is a fixed request shape with the model filling in a few
 * fields, rather than one `call_google_api` that takes a URL. A generic caller
 * is smaller code and a much larger blast radius: the grant would reach every
 * endpoint the scopes allow rather than the handful this file names, and
 * nothing in a review could tell you which. Cloudflare's own MCP server makes
 * the opposite trade for a defensible reason — 2,500 endpoints behind `search`
 * and `execute` — and it is not this one. Three Google APIs are small enough to
 * write out.
 */

import type { Provider } from "./connectors";
import { parseScopes } from "./connectors";

const GMAIL = "https://gmail.googleapis.com/gmail/v1/users/me";
/** The same resource, reached the way Google wants a message with bytes in it. */
const GMAIL_UPLOAD = "https://gmail.googleapis.com/upload/gmail/v1/users/me";
const CALENDAR = "https://www.googleapis.com/calendar/v3";
const DRIVE = "https://www.googleapis.com/drive/v3";
const DRIVE_UPLOAD = "https://www.googleapis.com/upload/drive/v3";

/**
 * How much attached file a send may carry, counted before anything is fetched.
 *
 * This is our limit, not Gmail's: Gmail accepts a 25 MB message, and a Worker
 * has 128 MB of memory for the whole request. Assembling a message means
 * holding the base64 of every file, the wrapped copy of it and the joined
 * message at once, so the ceiling is several times whatever this number is.
 * Raising it means streaming the parts into a resumable upload instead of
 * building a string, which is a different piece of code.
 *
 * Counted from the sizes Gmail already reported on the message, so a request
 * that is too big is refused without transferring a byte.
 */
const MAX_ATTACHED_BYTES = 10 * 1024 * 1024;

/** A ceiling on fetches per send, so one call cannot become a hundred. */
const MAX_ATTACHMENTS = 10;

/** What a tool call answers with: text a model reads, or a refusal it can act on. */
export interface ToolResult {
  text: string;
  isError?: boolean;
}

export interface Tool {
  name: string;
  description: string;
  /** JSON Schema, handed to the model as-is. */
  inputSchema: Record<string, unknown>;
  /** Every entry is required; an array accepts any one of its scopes. */
  scopes: readonly (string | readonly string[])[];
  run: (token: string, args: Record<string, unknown>) => Promise<ToolResult>;
}

const str = (args: Record<string, unknown>, key: string): string | undefined => {
  const value = args[key];
  return typeof value === "string" && value.trim() ? value.trim() : undefined;
};

const num = (args: Record<string, unknown>, key: string): number | undefined => {
  const value = args[key];
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
};

/**
 * One call to a provider, with the failure turned into something readable.
 *
 * A provider's error body is JSON with the useful sentence buried in it, and
 * handing a model the raw envelope wastes a turn. A refusal comes back as a
 * `ToolResult` rather than a thrown error because the two mean different things
 * to an agent: "that call was wrong" is worth rewording and retrying, and "the
 * plugin is unreachable" is not.
 */
async function callProvider(
  token: string,
  url: string,
  init: RequestInit = {},
): Promise<{ ok: true; body: unknown } | { ok: false; status: number; text: string }> {
  let res: Response;
  try {
    res = await fetch(url, {
      ...init,
      headers: {
        authorization: `Bearer ${token}`,
        "content-type": "application/json",
        ...(init.headers as Record<string, string> | undefined),
      },
    });
  } catch (error) {
    // Status 0 for "never reached the provider", which is not any HTTP answer
    // and is the one failure a caller cannot explain to an operator.
    return { ok: false, status: 0, text: `Could not reach the provider: ${String(error)}` };
  }

  const raw = await res.text();
  let body: unknown;
  try {
    body = raw ? JSON.parse(raw) : null;
  } catch {
    body = raw;
  }

  if (!res.ok) {
    const message =
      (body as { error?: { message?: string } } | null)?.error?.message ??
      (typeof body === "string" ? body.slice(0, 300) : "") ??
      "";
    // The status is named because 403 and 404 mean different things to an
    // agent: one is a scope it does not have and will never have this turn,
    // the other is a thing that is not there.
    return {
      ok: false,
      status: res.status,
      text: `The provider refused with HTTP ${res.status}. ${message}`.trim(),
    };
  }
  return { ok: true, body };
}

const ok = (body: unknown): ToolResult => ({
  text: typeof body === "string" ? body : JSON.stringify(body, null, 2),
});

const refuse = (text: string): ToolResult => ({ text, isError: true });

/**
 * A header value that cannot become two headers.
 *
 * `to` and `subject` are written by a model, and a model can be talked into
 * things. A CR or LF inside either one ends the header and starts another, so
 * a subject reading `Notes\r\nBcc: elsewhere@example.com` would silently copy
 * the mail to a third party. Stripping is right rather than refusing: the
 * newline is never what the caller meant, and a refusal here would be a puzzle
 * for an agent that did nothing wrong.
 */
const headerValue = (value: string): string => value.replace(/[\r\n]+/g, " ").trim();

/**
 * A boundary that content cannot forge.
 *
 * Both bodies here are model-supplied, and a fixed boundary means a body
 * containing that string splits the message: extra parts, or metadata the
 * caller was never offered a field for. A random one per request cannot be
 * guessed by whatever wrote the content.
 */
const boundary = () => `guaca-${crypto.randomUUID()}`;

/** One file on its way out, holding Gmail's own base64url rather than bytes. */
interface Attached {
  filename: string;
  mimeType: string;
  /** base64url, exactly as Gmail returned it. Never decoded on this path. */
  data: string;
}

/**
 * base64url to padded base64, wrapped to 76 columns.
 *
 * Gmail hands attachment content out in base64url and MIME wants base64, which
 * is two character substitutions and the padding Gmail omits. The bytes are
 * never decoded: a round trip through a Uint8Array would trade a string copy
 * for two, and the point of this path is that a file passes through the Worker
 * without ever being materialized as bytes.
 *
 * The wrap is not cosmetic. RFC 5322 caps a line at 998 characters, so a
 * megabyte of base64 on one line is a message some hop is entitled to reject.
 */
function mimeBase64(data: string): string {
  const standard = data.replace(/-/g, "+").replace(/_/g, "/").replace(/=+$/, "");
  const padded = standard + "=".repeat((4 - (standard.length % 4)) % 4);
  return padded.replace(/.{76}/g, "$&\r\n").replace(/\r\n$/, "");
}

/**
 * `filename=` for a name a person actually gave a file.
 *
 * Quotes and newlines come out because they would end the parameter early. A
 * name outside ASCII gets both forms: a transliterated `filename` that every
 * mailer understands and the RFC 2231 `filename*` that carries the real one,
 * which is what Gmail itself sends.
 */
function filenameParams(name: string): string {
  const safe = name.replace(/[\r\n"\\]/g, "").trim() || "attachment";
  const printable = /^[\u0020-\u007e]+$/;
  if (printable.test(safe)) return `filename="${safe}"`;
  const fallback = safe.replace(/[^\u0020-\u007e]/g, "_");
  return `filename="${fallback}"; filename*=UTF-8''${encodeURIComponent(safe)}`;
}

/** RFC 2822: one text part, or a `multipart/mixed` when files came along. */
function buildMessage(to: string, subject: string, body: string, files: Attached[]): string {
  const head = [`To: ${headerValue(to)}`, `Subject: ${headerValue(subject)}`, "MIME-Version: 1.0"];
  if (files.length === 0) {
    return [...head, "Content-Type: text/plain; charset=UTF-8", "", body].join("\r\n");
  }

  const mark = boundary();
  const lines = [
    ...head,
    `Content-Type: multipart/mixed; boundary="${mark}"`,
    "",
    `--${mark}`,
    "Content-Type: text/plain; charset=UTF-8",
    "",
    body,
  ];
  for (const file of files) {
    lines.push(
      `--${mark}`,
      `Content-Type: ${headerValue(file.mimeType) || "application/octet-stream"}`,
      `Content-Disposition: attachment; ${filenameParams(file.filename)}`,
      "Content-Transfer-Encoding: base64",
      "",
      mimeBase64(file.data),
    );
  }
  lines.push(`--${mark}--`, "");
  return lines.join("\r\n");
}

/** base64url of a whole message, which is what Gmail's JSON send takes. */
function base64urlMessage(message: string): string {
  const bytes = new TextEncoder().encode(message);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Gmail's `body` on a part: content inline, or a receipt for fetching it. */
interface PartBody {
  data?: string;
  attachmentId?: string;
  size?: number;
}

/** Every part of a payload, depth first, the payload itself included. */
function* walkParts(part: Record<string, unknown>): Generator<Record<string, unknown>> {
  yield part;
  for (const child of (part.parts ?? []) as Record<string, unknown>[]) yield* walkParts(child);
}

/**
 * The parts a person would call attachments: the ones with a name of their own.
 *
 * Gmail sets `filename` to the empty string on the parts that make up the
 * message body and to the real name on everything that arrived alongside it,
 * which is the same line a mail client draws. Inline images from HTML mail land
 * on this side of it, correctly: they are files, and a caller can tell from the
 * name and the type.
 */
function attachedParts(payload: Record<string, unknown>): Record<string, unknown>[] {
  return [...walkParts(payload)].filter(
    (part) => typeof part.filename === "string" && part.filename !== "",
  );
}

/**
 * Gmail's own message shape, reduced to what an agent asked for.
 *
 * The API answers with a `payload` tree of MIME parts, base64url bodies and a
 * header array. Handing that to a model costs hundreds of tokens per message
 * and it still has to walk the tree, so the walk happens here once.
 *
 * Attachments are listed and not decoded. A file is named, typed, sized and
 * given the `partId` that `gmail_send` takes, which is everything needed to
 * forward it and none of its bytes: a 6 MB deck is 8 MB of base64 that no model
 * can read and every model would have to pay for.
 */
function readMessage(message: Record<string, unknown>): Record<string, unknown> {
  const payload = (message.payload ?? {}) as Record<string, unknown>;
  const headers = (payload.headers ?? []) as { name?: string; value?: string }[];
  const header = (name: string) =>
    headers.find((h) => h.name?.toLowerCase() === name.toLowerCase())?.value ?? "";

  const decode = (data: string) => {
    try {
      const normal = data.replace(/-/g, "+").replace(/_/g, "/");
      const binary = atob(normal);
      const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
      return new TextDecoder().decode(bytes);
    } catch {
      return "";
    }
  };

  // Depth-first for the first text/plain part that is not itself a file,
  // falling back to the top-level body. A multipart/alternative message puts
  // plain text beside the HTML and the plain one is what a model should read;
  // an attached .txt is not the message and must not be mistaken for it.
  const bodyPart = [...walkParts(payload)].find(
    (part) => part.mimeType === "text/plain" && !part.filename && (part.body as PartBody)?.data,
  );

  const text =
    decode(((bodyPart?.body ?? {}) as PartBody).data ?? "") ||
    decode(((payload.body ?? {}) as PartBody).data ?? "");

  const attachments = attachedParts(payload).map((part) => ({
    partId: part.partId,
    filename: part.filename,
    mimeType: part.mimeType,
    size: (part.body as PartBody | undefined)?.size,
  }));

  return {
    id: message.id,
    threadId: message.threadId,
    from: header("From"),
    to: header("To"),
    subject: header("Subject"),
    date: header("Date"),
    snippet: message.snippet,
    body: text.slice(0, 20000),
    // Omitted rather than empty: most mail has none, and a `[]` on every
    // message is a line of noise in every search result that carries it.
    ...(attachments.length ? { attachments } : {}),
  };
}

/** What `gmail_send` is handed: a file, named by the message it arrived on. */
interface AttachRef {
  messageId: string;
  partId: string;
}

/**
 * `attach` as the model wrote it, or `null` if it is not the shape promised.
 *
 * Refusing an unparseable argument outright beats attaching nothing and
 * reporting success, which is how a model concludes it forwarded a file it did
 * not forward. `partId` is accepted as a number as well because Gmail's are
 * `0`, `1`, `1.1` and a model reading that list will sometimes send `1`.
 */
function attachRefs(value: unknown): AttachRef[] | null {
  if (value === undefined || value === null) return [];
  if (!Array.isArray(value)) return null;

  const refs: AttachRef[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object") return null;
    const record = item as Record<string, unknown>;
    const messageId = typeof record.messageId === "string" ? record.messageId.trim() : "";
    const rawPart = record.partId;
    const partId =
      typeof rawPart === "string"
        ? rawPart.trim()
        : typeof rawPart === "number" && Number.isFinite(rawPart)
          ? String(rawPart)
          : "";
    if (!messageId || !partId) return null;
    refs.push({ messageId, partId });
  }
  return refs;
}

/**
 * Turn references into files, reading each one out of the mail it arrived on.
 *
 * The name and the type come from the source message rather than from the
 * caller. That is the whole reason the argument is a reference: a model that
 * supplied its own `filename` could relabel a spreadsheet as a PDF, by mistake
 * or because something in the mail it just read told it to, and nothing
 * downstream would notice.
 *
 * Size is checked against what Gmail already reported, before a byte moves. A
 * refusal after four megabytes of transfer is the same refusal, later.
 */
async function resolveAttachments(
  token: string,
  refs: AttachRef[],
): Promise<{ ok: true; files: Attached[] } | { ok: false; text: string }> {
  if (refs.length > MAX_ATTACHMENTS) {
    return {
      ok: false,
      text: `That is ${refs.length} attachments and this tool sends at most ${MAX_ATTACHMENTS} in one message.`,
    };
  }

  // One read per message however many files came off it, because two
  // attachments on one mail are the common case.
  const messages = new Map<string, Record<string, unknown>>();
  for (const { messageId } of refs) {
    if (messages.has(messageId)) continue;
    const got = await callProvider(
      token,
      `${GMAIL}/messages/${encodeURIComponent(messageId)}?format=full`,
    );
    if (!got.ok) {
      // 403 here is a specific, fixable thing and worth naming: sending is one
      // half of the Gmail grant and reading is the other, so an account can
      // hold `gmail_send` and still be unable to attach anything to it.
      const hint =
        got.status === 403
          ? " Attaching a file that arrived by mail needs the read half of the Gmail grant, not only the send half."
          : "";
      return {
        ok: false,
        text: `Could not open message ${messageId} to attach from it. ${got.text}${hint}`,
      };
    }
    messages.set(messageId, got.body as Record<string, unknown>);
  }

  const chosen: { ref: AttachRef; part: Record<string, unknown> }[] = [];
  for (const ref of refs) {
    const payload = (messages.get(ref.messageId)?.payload ?? {}) as Record<string, unknown>;
    const part = attachedParts(payload).find((candidate) => candidate.partId === ref.partId);
    if (!part) {
      const named = attachedParts(payload)
        .map((candidate) => `${candidate.partId} (${candidate.filename})`)
        .join(", ");
      return {
        ok: false,
        text: `Message ${ref.messageId} has no attachment with partId ${ref.partId}. ${
          named ? `It has: ${named}.` : "It has no attachments at all."
        }`,
      };
    }
    chosen.push({ ref, part });
  }

  const total = chosen.reduce(
    (sum, { part }) => sum + ((part.body as PartBody | undefined)?.size ?? 0),
    0,
  );
  if (total > MAX_ATTACHED_BYTES) {
    const mb = (bytes: number) => (bytes / 1024 / 1024).toFixed(1);
    return {
      ok: false,
      text: `That is ${mb(total)} MB of attachments and this tool sends at most ${mb(MAX_ATTACHED_BYTES)} MB, which is a limit of the server rather than of Gmail. Send the files in more than one message, or share them another way.`,
    };
  }

  const files: Attached[] = [];
  for (const { ref, part } of chosen) {
    const body = (part.body ?? {}) as PartBody;
    let data = body.data;
    // Gmail inlines a small part and hands out a receipt for a large one, so
    // both are normal and only the second costs a request.
    if (!data && body.attachmentId) {
      const got = await callProvider(
        token,
        `${GMAIL}/messages/${encodeURIComponent(ref.messageId)}/attachments/${encodeURIComponent(body.attachmentId)}`,
      );
      if (!got.ok) {
        return { ok: false, text: `Could not read ${part.filename} to attach it. ${got.text}` };
      }
      data = (got.body as { data?: string } | null)?.data;
    }
    if (!data) {
      return { ok: false, text: `Gmail returned no content for ${part.filename}.` };
    }
    files.push({
      filename: String(part.filename),
      mimeType: typeof part.mimeType === "string" ? part.mimeType : "application/octet-stream",
      data,
    });
  }
  return { ok: true, files };
}

const GOOGLE_TOOLS: Tool[] = [
  {
    name: "gmail_search",
    description:
      "Search the operator's Gmail and return matching messages with their sender, subject, date and snippet. Use Gmail's own query syntax, for example 'from:someone@example.com is:unread newer_than:7d'. Returns ids to pass to gmail_read.",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "A Gmail search query." },
        limit: { type: "number", description: "How many to return, 1 to 25. Defaults to 10." },
      },
      required: ["query"],
    },
    scopes: ["https://www.googleapis.com/auth/gmail.readonly"],
    run: async (token, args) => {
      const query = str(args, "query");
      if (!query) return refuse("gmail_search needs a query.");
      const limit = Math.min(Math.max(num(args, "limit") ?? 10, 1), 25);

      const listed = await callProvider(
        token,
        `${GMAIL}/messages?q=${encodeURIComponent(query)}&maxResults=${limit}`,
      );
      if (!listed.ok) return refuse(listed.text);

      const ids = ((listed.body as { messages?: { id: string }[] } | null)?.messages ?? []).map(
        (m) => m.id,
      );
      if (ids.length === 0) return ok("No messages matched that query.");

      // Metadata rather than full bodies: a search result is for choosing which
      // message to open, and full bodies would be most of a context window.
      const messages = [];
      for (const id of ids) {
        const got = await callProvider(
          token,
          `${GMAIL}/messages/${id}?format=metadata&metadataHeaders=From&metadataHeaders=To&metadataHeaders=Subject&metadataHeaders=Date`,
        );
        if (got.ok) {
          const { body: _body, ...rest } = readMessage(got.body as Record<string, unknown>);
          messages.push(rest);
        }
      }
      return ok(messages);
    },
  },
  {
    name: "gmail_read",
    description:
      "Read one Gmail message in full, including its plain-text body. Takes an id from gmail_search. Any files on the message are listed under 'attachments' with their name, type, size and partId; the bytes are not returned, and the partId is what gmail_send takes to forward one.",
    inputSchema: {
      type: "object",
      properties: { id: { type: "string", description: "The message id." } },
      required: ["id"],
    },
    scopes: ["https://www.googleapis.com/auth/gmail.readonly"],
    run: async (token, args) => {
      const id = str(args, "id");
      if (!id) return refuse("gmail_read needs a message id, which gmail_search returns.");
      const got = await callProvider(
        token,
        `${GMAIL}/messages/${encodeURIComponent(id)}?format=full`,
      );
      if (!got.ok) return refuse(got.text);
      return ok(readMessage(got.body as Record<string, unknown>));
    },
  },
  {
    name: "gmail_send",
    description:
      "Send an email as the operator, optionally forwarding files that arrived on their mail. This acts as them and cannot be undone, so confirm the recipient and the wording before calling it. Attaching a file needs the read half of the Gmail grant as well as the send half.",
    inputSchema: {
      type: "object",
      properties: {
        to: { type: "string", description: "The recipient's email address." },
        subject: { type: "string" },
        body: { type: "string", description: "Plain text." },
        attach: {
          type: "array",
          description:
            "Files to attach, each named by the message it arrived on rather than by content. Their names and types are read from that message, so a file cannot be relabelled on the way through.",
          items: {
            type: "object",
            properties: {
              messageId: {
                type: "string",
                description: "The id of the message the file arrived on, from gmail_search.",
              },
              partId: {
                type: "string",
                description: "The file's partId, from that message's gmail_read attachments list.",
              },
            },
            required: ["messageId", "partId"],
          },
        },
      },
      required: ["to", "subject", "body"],
    },
    scopes: ["https://www.googleapis.com/auth/gmail.send"],
    run: async (token, args) => {
      const to = str(args, "to");
      const subject = str(args, "subject");
      const body = typeof args.body === "string" ? args.body : undefined;
      if (!to || !subject || body === undefined) {
        return refuse("gmail_send needs to, subject and body.");
      }

      const refs = attachRefs(args.attach);
      if (!refs) {
        return refuse(
          "gmail_send's attach must be a list of {messageId, partId}, both from gmail_read.",
        );
      }

      const resolved = await resolveAttachments(token, refs);
      if (!resolved.ok) return refuse(`Nothing was sent. ${resolved.text}`);
      const { files } = resolved;
      const message = buildMessage(to, subject, body, files);

      // Two endpoints for one send. The JSON one takes the whole message
      // base64url'd inside a field, which is fine for text and wasteful for
      // bytes; the upload one takes the message itself and is what Google
      // documents for anything with a file on it.
      const sent = files.length
        ? await callProvider(token, `${GMAIL_UPLOAD}/messages/send?uploadType=media`, {
            method: "POST",
            headers: { "content-type": "message/rfc822" },
            body: message,
          })
        : await callProvider(token, `${GMAIL}/messages/send`, {
            method: "POST",
            body: JSON.stringify({ raw: base64urlMessage(message) }),
          });
      if (!sent.ok) return refuse(sent.text);

      const names = files.map((file) => file.filename).join(", ");
      return ok(names ? `Sent to ${to} with ${names}.` : `Sent to ${to}.`);
    },
  },
  {
    name: "calendar_list_events",
    description:
      "List events from the operator's primary calendar in a time range. Times are RFC 3339, for example 2026-08-24T09:00:00Z.",
    inputSchema: {
      type: "object",
      properties: {
        from: { type: "string", description: "Start of the range, RFC 3339. Defaults to now." },
        to: { type: "string", description: "End of the range, RFC 3339." },
        limit: { type: "number", description: "How many to return, 1 to 50. Defaults to 20." },
      },
    },
    scopes: ["https://www.googleapis.com/auth/calendar"],
    run: async (token, args) => {
      const limit = Math.min(Math.max(num(args, "limit") ?? 20, 1), 50);
      const params = new URLSearchParams({
        singleEvents: "true",
        orderBy: "startTime",
        maxResults: String(limit),
      });
      // The API needs a lower bound to order by start time, and "now" is the
      // range a person means when they do not say.
      params.set("timeMin", str(args, "from") ?? new Date().toISOString());
      const to = str(args, "to");
      if (to) params.set("timeMax", to);

      const listed = await callProvider(token, `${CALENDAR}/calendars/primary/events?${params}`);
      if (!listed.ok) return refuse(listed.text);

      const events = (
        (listed.body as { items?: Record<string, unknown>[] } | null)?.items ?? []
      ).map((event) => ({
        id: event.id,
        summary: event.summary,
        start: event.start,
        end: event.end,
        location: event.location,
        attendees: ((event.attendees ?? []) as { email?: string }[]).map((a) => a.email),
      }));
      return events.length === 0 ? ok("No events in that range.") : ok(events);
    },
  },
  {
    name: "calendar_create_event",
    description:
      "Create an event on the operator's primary calendar. Times are RFC 3339. Attendees are invited and will be emailed by Google.",
    inputSchema: {
      type: "object",
      properties: {
        summary: { type: "string", description: "The event title." },
        start: { type: "string", description: "RFC 3339." },
        end: { type: "string", description: "RFC 3339." },
        description: { type: "string" },
        location: { type: "string" },
        attendees: {
          type: "array",
          items: { type: "string" },
          description: "Email addresses to invite.",
        },
      },
      required: ["summary", "start", "end"],
    },
    scopes: ["https://www.googleapis.com/auth/calendar"],
    run: async (token, args) => {
      const summary = str(args, "summary");
      const start = str(args, "start");
      const end = str(args, "end");
      if (!summary || !start || !end) {
        return refuse("calendar_create_event needs summary, start and end.");
      }
      const attendees = Array.isArray(args.attendees)
        ? (args.attendees as unknown[]).filter((a): a is string => typeof a === "string")
        : [];

      const made = await callProvider(token, `${CALENDAR}/calendars/primary/events`, {
        method: "POST",
        body: JSON.stringify({
          summary,
          start: { dateTime: start },
          end: { dateTime: end },
          ...(str(args, "description") ? { description: str(args, "description") } : {}),
          ...(str(args, "location") ? { location: str(args, "location") } : {}),
          ...(attendees.length ? { attendees: attendees.map((email) => ({ email })) } : {}),
        }),
      });
      if (!made.ok) return refuse(made.text);
      const event = made.body as { id?: string; htmlLink?: string };
      return ok(`Created event ${event.id}. ${event.htmlLink ?? ""}`.trim());
    },
  },
  {
    name: "calendar_delete_event",
    description:
      "Cancel an event on the operator's primary calendar. Attendees are notified by Google. Takes an id from calendar_list_events.",
    inputSchema: {
      type: "object",
      properties: { id: { type: "string", description: "The event id." } },
      required: ["id"],
    },
    scopes: ["https://www.googleapis.com/auth/calendar"],
    run: async (token, args) => {
      const id = str(args, "id");
      if (!id) return refuse("calendar_delete_event needs an event id.");
      const gone = await callProvider(
        token,
        `${CALENDAR}/calendars/primary/events/${encodeURIComponent(id)}`,
        { method: "DELETE" },
      );
      if (!gone.ok) return refuse(gone.text);
      return ok(`Cancelled event ${id}.`);
    },
  },
  {
    name: "drive_list_files",
    description:
      "List accessible Drive files. With drive.readonly, this includes existing files across the operator's Drive; older drive.file grants only see files shared with or created by Guaca.",
    inputSchema: {
      type: "object",
      properties: {
        query: {
          type: "string",
          description: "Optional Drive query, for example \"name contains 'notes'\".",
        },
        limit: { type: "number", description: "How many to return, 1 to 50. Defaults to 20." },
      },
    },
    scopes: [
      [
        "https://www.googleapis.com/auth/drive.file",
        "https://www.googleapis.com/auth/drive.readonly",
      ],
    ],
    run: async (token, args) => {
      const limit = Math.min(Math.max(num(args, "limit") ?? 20, 1), 50);
      const params = new URLSearchParams({
        pageSize: String(limit),
        fields: "files(id,name,mimeType,modifiedTime,webViewLink)",
      });
      const query = str(args, "query");
      if (query) params.set("q", query);

      const listed = await callProvider(token, `${DRIVE}/files?${params}`);
      if (!listed.ok) return refuse(listed.text);
      const files = (listed.body as { files?: unknown[] } | null)?.files ?? [];
      return files.length === 0 ? ok("No files matched.") : ok(files);
    },
  },
  {
    name: "drive_create_file",
    description:
      "Create a plain-text file in the operator's Drive. Returns an id and a link. The file belongs to them; this app can reopen it later because it created it.",
    inputSchema: {
      type: "object",
      properties: {
        name: { type: "string", description: "The file name, including an extension." },
        content: { type: "string", description: "The file's text." },
      },
      required: ["name", "content"],
    },
    scopes: ["https://www.googleapis.com/auth/drive.file"],
    run: async (token, args) => {
      const name = str(args, "name");
      const content = typeof args.content === "string" ? args.content : undefined;
      if (!name || content === undefined)
        return refuse("drive_create_file needs name and content.");

      // Multipart, because metadata and bytes go in one request and Drive
      // rejects a body that sends only one of them. The boundary is random for
      // the same reason it is in a mail: `content` is written by a model, and a
      // fixed one could be reproduced inside the file to open a second metadata
      // part the caller was never offered a field for.
      const mark = boundary();
      const body = [
        `--${mark}`,
        "Content-Type: application/json; charset=UTF-8",
        "",
        JSON.stringify({ name, mimeType: "text/plain" }),
        `--${mark}`,
        "Content-Type: text/plain; charset=UTF-8",
        "",
        content,
        `--${mark}--`,
        "",
      ].join("\r\n");

      const made = await callProvider(
        token,
        `${DRIVE_UPLOAD}/files?uploadType=multipart&fields=id,name,webViewLink`,
        {
          method: "POST",
          headers: { "content-type": `multipart/related; boundary=${mark}` },
          body,
        },
      );
      if (!made.ok) return refuse(made.text);
      const file = made.body as { id?: string; webViewLink?: string };
      return ok(`Created ${name} (${file.id}). ${file.webViewLink ?? ""}`.trim());
    },
  },
  {
    name: "drive_read_file",
    description:
      "Read the text of an accessible Drive file. Takes an id from drive_list_files or drive_create_file.",
    inputSchema: {
      type: "object",
      properties: { id: { type: "string", description: "The file id." } },
      required: ["id"],
    },
    scopes: [
      [
        "https://www.googleapis.com/auth/drive.file",
        "https://www.googleapis.com/auth/drive.readonly",
      ],
    ],
    run: async (token, args) => {
      const id = str(args, "id");
      if (!id) return refuse("drive_read_file needs a file id.");
      const got = await callProvider(token, `${DRIVE}/files/${encodeURIComponent(id)}?alt=media`);
      if (!got.ok) return refuse(got.text);
      return ok(typeof got.body === "string" ? got.body.slice(0, 40000) : got.body);
    },
  },
];

const BY_PROVIDER: Record<string, Tool[]> = { google: GOOGLE_TOOLS };

/**
 * The tools one grant actually supports.
 *
 * Keyed on the scopes the provider returned rather than on the capabilities the
 * operator ticked, because those are not the same thing: a consent screen lets
 * someone untick a box on the way through, and a grant can be narrowed later.
 */
export function toolsFor(provider: Provider, scope: string | null | undefined): Tool[] {
  const held = parseScopes(scope);
  return (BY_PROVIDER[provider.id] ?? []).filter((tool) =>
    tool.scopes.every((needed) =>
      typeof needed === "string" ? held.has(needed) : needed.some((scope) => held.has(scope)),
    ),
  );
}

/** Every tool this origin could ever offer, for a test that walks them. */
export function allTools(): Tool[] {
  return Object.values(BY_PROVIDER).flat();
}
