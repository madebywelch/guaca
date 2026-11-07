/**
 * One authorized identity at one provider, which is the unit everything here
 * actually deals in.
 *
 * ## Why this replaced "one grant per provider"
 *
 * That was the original claim and it was wrong by one word. Google and GitHub
 * hold one grant per *(person, OAuth client, identity)*, not one per provider:
 * revoking a token revokes that identity's grant and says nothing about
 * another. Better Auth models it exactly that way, keying `account` on
 * `(issuer, accountId)` where `accountId` is the provider's subject. Two Google
 * accounts are two subjects and therefore two rows, and always have been.
 *
 * What was missing was everything above the table. `readGrants` took the first
 * row matching a provider, the page drew one card, and the MCP server offered
 * one grant's tools. So a person with a work Google and a personal one could
 * authorize the second and never see it again, and a crew had no way to name
 * which of the two it meant.
 *
 * A connection is that row, surfaced. It has an id a group can hold onto and a
 * label a person can tell apart from the other one.
 *
 * ## Where the label comes from
 *
 * The id token, decrypted and read. Better Auth stores one on the row when the
 * provider issues one, encrypted with everything else, and Google always issues
 * one because `identityScopes` asks for `openid`. That is a local read rather
 * than a call to the provider, which matters: listing connections happens on
 * every page load and on every `/api/connectors`, and N round trips to Google
 * to render N labels would be paid by a desktop app polling in the background.
 *
 * A provider that issues no id token gets its subject as a label instead.
 * GitHub is that provider today. It is honest rather than pretty, and it is not
 * worth a network call until GitHub has tools worth choosing between.
 */

import { decryptOAuthToken } from "better-auth/oauth2";
import { eq } from "drizzle-orm";
import { drizzle } from "drizzle-orm/d1";

import type { Auth } from "./auth";
import { grantedCapabilities, type Provider } from "./connectors";
import { account, schema } from "./schema";

/** What `decryptOAuthToken` needs, which is a slice of Better Auth's context. */
type DecryptContext = Parameters<typeof decryptOAuthToken>[1];

export interface Connection {
  /** The `account` row id. Stable, and what a group stores to name this one. */
  id: string;
  providerId: string;
  /**
   * Who this is at the provider, for a person choosing between two of them.
   * The provider's email where it says, and its subject where it does not.
   */
  label: string;
  /** Exactly what the provider returned, which is not what was asked for. */
  scope: string | null;
  /** The same scopes, read into the capabilities the catalogue names. */
  capabilities: Set<string>;
}

/**
 * Every identity this person has authorized, newest last.
 *
 * Ordered by when it was linked so a list is stable across reads: a page that
 * reordered its cards between two loads would move the button somebody was
 * about to click.
 */
export async function readConnections(
  env: Env,
  auth: Auth,
  userId: string,
  providers: readonly Provider[],
): Promise<Connection[]> {
  const db = drizzle(env.DB, { schema });
  const rows = await db.select().from(account).where(eq(account.userId, userId));
  // Cast at this one boundary. `decryptOAuthToken` wants Better Auth's own
  // `AuthContext`, and the context a configured instance hands back is a
  // narrower generic that TypeScript will not widen to it. The function reads
  // two fields off it and both are present; the alternative is threading the
  // mismatch through every caller.
  const context = (await auth.$context) as unknown as DecryptContext;

  const byId = new Map(providers.map((provider) => [provider.id, provider]));
  const out: Connection[] = [];

  for (const row of rows) {
    const provider = byId.get(row.providerId);
    if (!provider) continue;
    // A row with no scope is what a completed disconnect leaves behind:
    // `clearGrantTokens` nulls the tokens and the scope and keeps the row,
    // because the row is also how a person who signed in with that provider
    // gets back in. It is not a connection, and drawing it as one would offer a
    // card with nothing authorized and a disconnect button for a grant that is
    // already gone.
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

/** One connection by id, and only if it belongs to this person. */
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

/**
 * The provider's own name for whoever authorized this.
 *
 * Failures fall back to the subject rather than throwing. An id token that
 * cannot be decrypted is a row written under a different `BETTER_AUTH_SECRET`,
 * which is a real state after a rotation, and a page that 500s on it is worse
 * than one that shows an ugly label beside a working disconnect button.
 */
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
