/**
 * This origin as an MCP server, so a crew can reach what the account holds.
 *
 * ## Why this exists rather than a second API
 *
 * `/api/connectors` already tells a paired machine what is authorized, and
 * `/api/connectors/:provider/token` already hands it an access token. Guaca
 * could have called Google itself with that token. It should not, and the
 * reason is the app's own boundary rather than anything about this origin: a
 * plugin's tools are called *by Guaca* on behalf of an agent, and the grant
 * never enters a prompt, a transcript, an event or a sandbox's environment.
 * Vending a live Google token down to the desktop puts a working credential on
 * a laptop for an hour at a time and makes the app responsible for it. Serving
 * tools instead keeps the token here, where the refresh token already lives.
 *
 * It also means the app needs no new machinery. Guaca already speaks MCP,
 * already reads a server's tool list once and keeps it, already decides which
 * agents in a crew may call a plugin, and already offers those tools to a model
 * with the schemas the server published. This becomes one more server on that
 * list, and every one of those behaviours comes for free.
 *
 * ## What authenticates a call
 *
 * The account's own OAuth access token, the same bearer `/api/connectors`
 * takes, carrying the `connectors` scope. Not a per-group grant: the account is
 * the thing that holds the Google grant, and asking an operator to complete a
 * second OAuth dance to reach their own account would be a consent screen for
 * something they already consented to.
 *
 * ## The protocol, reduced
 *
 * Three methods and one notification, which is exactly what Guaca's client
 * sends: `initialize`, `notifications/initialized`, `tools/list`, `tools/call`.
 * No session id is issued, because there is no session state to key: every call
 * re-reads the grant. A client that sends `Mcp-Session-Id` anyway is not
 * refused, it is ignored, which is what the specification asks of a server that
 * issued none.
 */

import type { Auth } from "./auth";
import { readConnections } from "./connections";
import { configuredProviders, findProvider } from "./connectors";
import { type Tool, toolsFor } from "./tools";

/** How this server names itself on `initialize`. */
export const SERVER_NAME = "Guaca Connectors";

export const PROTOCOL_VERSION = "2025-06-18";

interface Request {
  jsonrpc?: string;
  id?: unknown;
  method?: string;
  params?: Record<string, unknown>;
}

const result = (id: unknown, value: unknown) =>
  Response.json({ jsonrpc: "2.0", id, result: value });

/**
 * A JSON-RPC error, which is not the same as a tool refusing.
 *
 * This one means the request itself was wrong: an unknown method, a tool that
 * does not exist. A tool that ran and said no comes back as a normal result
 * with `isError`, because an agent can act on that and Guaca's client renders
 * it as the tool's answer rather than as the plugin being unreachable.
 */
const failed = (id: unknown, code: number, message: string) =>
  Response.json({ jsonrpc: "2.0", id, error: { code, message } });

/** Who the bearer turned out to be, as `worker/index.ts` resolved it. */
export interface Caller {
  userId: string;
  /**
   * Which authorized identity this call is for, when the path named one.
   *
   * Absent is every connection at once, which is what bare `/mcp` means and
   * what a plugin connected before connections existed still dials. Present is
   * one, and a group holds the id so two crews can use two Google accounts.
   */
  connection?: string;
}

/**
 * Everything this caller may call right now, with the provider each belongs to.
 *
 * Read on every request rather than cached. A grant changes when the operator
 * authorizes or disconnects something in a browser, and this origin is the one
 * place that knows: a cached list here would be a tool a model is offered and
 * a 403 it cannot explain. Guaca caches the list its end, deliberately, and
 * reconnecting is what refreshes it.
 */
async function available(
  env: Env,
  auth: Auth,
  caller: Caller,
): Promise<{ tool: Tool; providerId: string; accountId: string }[]> {
  const providers = configuredProviders(env as unknown as Record<string, unknown>);
  const all = await readConnections(env, auth, caller.userId, providers);

  // Named, or all of them. A named connection that is not this person's simply
  // matches nothing, which is the same answer as one that does not exist: there
  // is no way to probe for somebody else's ids from here.
  const chosen = caller.connection
    ? all.filter((connection) => connection.id === caller.connection)
    : all;

  const byId = new Map(providers.map((provider) => [provider.id, provider]));
  const out: { tool: Tool; providerId: string; accountId: string }[] = [];
  const seen = new Set<string>();

  for (const connection of chosen) {
    const provider = byId.get(connection.providerId);
    if (!provider) continue;
    for (const tool of toolsFor(provider, connection.scope)) {
      // Two connections for one provider publish the same tool names, and a
      // model handed `gmail_search` twice cannot tell which account it would
      // reach. Bare `/mcp` therefore offers the first connection's copy, which
      // is the behaviour that existed before connections did; a crew that wants
      // the other one names it in the path.
      if (seen.has(tool.name)) continue;
      seen.add(tool.name);
      out.push({ tool, providerId: provider.id, accountId: connection.id });
    }
  }
  return out;
}

/**
 * A live provider token, refreshed here if it had expired.
 *
 * No headers on the call: `getAccessToken` resolves the user from a session
 * when a request carries one and refuses outright when a request carries
 * headers with no session in it, which is every call from a paired machine.
 * Naming the user is the server-side path, and the account id came from this
 * person's own grants, so it cannot address somebody else's row.
 */
async function providerToken(
  auth: Auth,
  userId: string,
  accountId: string,
): Promise<string | null> {
  try {
    const token = await auth.api.getAccessToken({ body: { accountId, userId } });
    return token.accessToken ?? null;
  } catch {
    return null;
  }
}

export async function handle(
  request: globalThis.Request,
  env: Env,
  auth: Auth,
  caller: Caller,
): Promise<Response> {
  let body: Request;
  try {
    body = (await request.json()) as Request;
  } catch {
    return failed(null, -32700, "That was not JSON.");
  }

  const { id, method } = body;

  switch (method) {
    case "initialize":
      return result(id, {
        protocolVersion: PROTOCOL_VERSION,
        capabilities: { tools: {} },
        serverInfo: { name: SERVER_NAME, version: "1" },
      });

    // A notification: no id, and the specification wants no body back. Guaca's
    // client sends it and ignores the answer, but a server that answered with a
    // JSON-RPC envelope for a request that has no id would be wrong.
    case "notifications/initialized":
      return new Response(null, { status: 202 });

    case "tools/list": {
      const tools = await available(env, auth, caller);
      return result(id, {
        tools: tools.map(({ tool }) => ({
          name: tool.name,
          description: tool.description,
          inputSchema: tool.inputSchema,
        })),
      });
    }

    case "tools/call": {
      const name = typeof body.params?.name === "string" ? body.params.name : "";
      const args =
        body.params?.arguments && typeof body.params.arguments === "object"
          ? (body.params.arguments as Record<string, unknown>)
          : {};

      const tools = await available(env, auth, caller);
      const found = tools.find((entry) => entry.tool.name === name);
      if (!found) {
        // Not a JSON-RPC error. A tool can vanish between a crew connecting the
        // plugin and an agent calling it — the operator disconnects Google, or
        // narrows the grant — and the agent needs a sentence it can act on
        // rather than a transport failure it will retry.
        return result(id, {
          isError: true,
          content: [
            {
              type: "text",
              text: `There is no tool called ${name || "(unnamed)"} on this account. It may have been available when this plugin was connected and not any more: ask the operator to check what is authorized at guaca.bot and reconnect the plugin.`,
            },
          ],
        });
      }

      const provider = findProvider(found.providerId);
      const token = await providerToken(auth, caller.userId, found.accountId);
      if (!token) {
        return result(id, {
          isError: true,
          content: [
            {
              type: "text",
              text: `The ${provider?.label ?? found.providerId} sign-in behind this tool has expired or been revoked. Ask the operator to authorize it again at guaca.bot; nothing you can do from here will fix it.`,
            },
          ],
        });
      }

      const answer = await found.tool.run(token, args);
      return result(id, {
        content: [{ type: "text", text: answer.text }],
        ...(answer.isError ? { isError: true } : {}),
      });
    }

    default:
      return failed(id ?? null, -32601, `This server does not implement ${method ?? "that"}.`);
  }
}
