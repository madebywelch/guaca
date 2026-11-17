import type { Auth } from "./auth";
import { readConnections } from "./connections";
import { configuredProviders, findProvider } from "./connectors";
import { type Tool, toolsFor } from "./tools";

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

const failed = (id: unknown, code: number, message: string) =>
  Response.json({ jsonrpc: "2.0", id, error: { code, message } });

export interface Caller {
  userId: string;
  connection?: string;
}

async function available(
  env: Env,
  caller: Caller,
): Promise<{ tool: Tool; providerId: string; accountId: string }[]> {
  const providers = configuredProviders(env as unknown as Record<string, unknown>);
  const all = await readConnections(env, caller.userId, providers);

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
      if (seen.has(tool.name)) continue;
      seen.add(tool.name);
      out.push({ tool, providerId: provider.id, accountId: connection.id });
    }
  }
  return out;
}

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

    case "notifications/initialized":
      return new Response(null, { status: 202 });

    case "tools/list": {
      const tools = await available(env, caller);
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

      const tools = await available(env, caller);
      const found = tools.find((entry) => entry.tool.name === name);
      if (!found) {
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
