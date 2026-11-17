import type { Provider } from "./connectors";
import { parseScopes } from "./connectors";

const GMAIL = "https://gmail.googleapis.com/gmail/v1/users/me";
const GMAIL_UPLOAD = "https://gmail.googleapis.com/upload/gmail/v1/users/me";
const CALENDAR = "https://www.googleapis.com/calendar/v3";
const DRIVE = "https://www.googleapis.com/drive/v3";
const DRIVE_UPLOAD = "https://www.googleapis.com/upload/drive/v3";

const MAX_ATTACHED_BYTES = 10 * 1024 * 1024;

const MAX_ATTACHMENTS = 10;

export interface ToolResult {
  text: string;
  isError?: boolean;
}

export interface Tool {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
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

const headerValue = (value: string): string => value.replace(/[\r\n]+/g, " ").trim();

const boundary = () => `guaca-${crypto.randomUUID()}`;

interface Attached {
  filename: string;
  mimeType: string;
  data: string;
}

function mimeBase64(data: string): string {
  const standard = data.replace(/-/g, "+").replace(/_/g, "/").replace(/=+$/, "");
  const padded = standard + "=".repeat((4 - (standard.length % 4)) % 4);
  return padded.replace(/.{76}/g, "$&\r\n").replace(/\r\n$/, "");
}

function filenameParams(name: string): string {
  const safe = name.replace(/[\r\n"\\]/g, "").trim() || "attachment";
  const printable = /^[\u0020-\u007e]+$/;
  if (printable.test(safe)) return `filename="${safe}"`;
  const fallback = safe.replace(/[^\u0020-\u007e]/g, "_");
  return `filename="${fallback}"; filename*=UTF-8''${encodeURIComponent(safe)}`;
}

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

function base64urlMessage(message: string): string {
  const bytes = new TextEncoder().encode(message);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

interface PartBody {
  data?: string;
  attachmentId?: string;
  size?: number;
}

function* walkParts(part: Record<string, unknown>): Generator<Record<string, unknown>> {
  yield part;
  for (const child of (part.parts ?? []) as Record<string, unknown>[]) yield* walkParts(child);
}

function attachedParts(payload: Record<string, unknown>): Record<string, unknown>[] {
  return [...walkParts(payload)].filter(
    (part) => typeof part.filename === "string" && part.filename !== "",
  );
}

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
    ...(attachments.length ? { attachments } : {}),
  };
}

interface AttachRef {
  messageId: string;
  partId: string;
}

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

  const messages = new Map<string, Record<string, unknown>>();
  for (const { messageId } of refs) {
    if (messages.has(messageId)) continue;
    const got = await callProvider(
      token,
      `${GMAIL}/messages/${encodeURIComponent(messageId)}?format=full`,
    );
    if (!got.ok) {
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

export function toolsFor(provider: Provider, scope: string | null | undefined): Tool[] {
  const held = parseScopes(scope);
  return (BY_PROVIDER[provider.id] ?? []).filter((tool) =>
    tool.scopes.every((needed) =>
      typeof needed === "string" ? held.has(needed) : needed.some((scope) => held.has(scope)),
    ),
  );
}

export function allTools(): Tool[] {
  return Object.values(BY_PROVIDER).flat();
}
