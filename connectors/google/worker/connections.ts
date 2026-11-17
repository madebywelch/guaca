import { eq } from "drizzle-orm";
import { drizzle } from "drizzle-orm/d1";

import { grantedCapabilities, type Provider } from "./connectors";
import { account, schema } from "./schema";

export interface Connection {
  id: string;
  providerId: string;
  label: string;
  scope: string | null;
  capabilities: Set<string>;
}

export async function readConnections(
  env: Env,
  userId: string,
  providers: readonly Provider[],
): Promise<Connection[]> {
  const db = drizzle(env.DB, { schema });
  const rows = await db.select().from(account).where(eq(account.userId, userId));

  const byId = new Map(providers.map((provider) => [provider.id, provider]));
  const out: Connection[] = [];

  for (const row of rows) {
    const provider = byId.get(row.providerId);
    if (!provider) continue;
    if (!row.scope) continue;
    out.push({
      id: row.id,
      providerId: provider.id,
      label: labelFor(row),
      scope: row.scope,
      capabilities: grantedCapabilities(provider, row.scope),
    });
  }

  out.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return out;
}

export async function readConnection(
  env: Env,
  userId: string,
  providers: readonly Provider[],
  id: string,
): Promise<Connection | undefined> {
  const all = await readConnections(env, userId, providers);
  return all.find((connection) => connection.id === id);
}

function labelFor(row: { label: string | null; accountId: string }): string {
  return row.label || row.accountId;
}

export function labelFromIdToken(idToken: string): string | undefined {
  const [, payload] = idToken.split(".");
  if (!payload) return undefined;
  try {
    const bytes = Uint8Array.from(atob(payload.replace(/-/g, "+").replace(/_/g, "/")), (char) =>
      char.charCodeAt(0),
    );
    const claims = JSON.parse(new TextDecoder().decode(bytes)) as { email?: unknown };
    return typeof claims.email === "string" && claims.email ? claims.email : undefined;
  } catch {
    return undefined;
  }
}
