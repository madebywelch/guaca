import { decryptOAuthToken } from "better-auth/oauth2";
import { eq } from "drizzle-orm";
import { drizzle } from "drizzle-orm/d1";

import type { Auth } from "./auth";
import { grantedCapabilities, type Provider } from "./connectors";
import { account, schema } from "./schema";

type DecryptContext = Parameters<typeof decryptOAuthToken>[1];

export interface Connection {
  id: string;
  providerId: string;
  label: string;
  scope: string | null;
  capabilities: Set<string>;
}

export async function readConnections(
  env: Env,
  auth: Auth,
  userId: string,
  providers: readonly Provider[],
): Promise<Connection[]> {
  const db = drizzle(env.DB, { schema });
  const rows = await db.select().from(account).where(eq(account.userId, userId));
  const context = (await auth.$context) as unknown as DecryptContext;

  const byId = new Map(providers.map((provider) => [provider.id, provider]));
  const out: Connection[] = [];

  for (const row of rows) {
    const provider = byId.get(row.providerId);
    if (!provider) continue;
    if (!row.scope) continue;
    out.push({
      id: row.id,
      providerId: provider.id,
      label: await labelFor(row.idToken, row.accountId, context),
      scope: row.scope,
      capabilities: grantedCapabilities(provider, row.scope),
    });
  }

  out.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return out;
}

export async function readConnection(
  env: Env,
  auth: Auth,
  userId: string,
  providers: readonly Provider[],
  id: string,
): Promise<Connection | undefined> {
  const all = await readConnections(env, auth, userId, providers);
  return all.find((connection) => connection.id === id);
}

async function labelFor(
  idToken: string | null,
  accountId: string,
  context: DecryptContext,
): Promise<string> {
  if (!idToken) return accountId;
  try {
    const decrypted = await decryptOAuthToken(idToken, context);
    if (!decrypted) return accountId;
    const [, payload] = decrypted.split(".");
    if (!payload) return accountId;
    const json = atob(payload.replace(/-/g, "+").replace(/_/g, "/"));
    const claims = JSON.parse(json) as { email?: unknown };
    return typeof claims.email === "string" && claims.email ? claims.email : accountId;
  } catch {
    return accountId;
  }
}
