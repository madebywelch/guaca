import { eq } from "drizzle-orm";
import { drizzle } from "drizzle-orm/d1";

import { account, schema } from "./schema";

/**
 * Reading a grant lives in `connections.ts`, not here.
 *
 * This file kept three readers that all took the first row matching a provider,
 * which is the assumption that made a second Google account invisible. They are
 * gone rather than deprecated: a reader that answers the wrong question is worse
 * than no reader, because it looks like the one you wanted.
 */

/**
 * Forget a grant, row and all.
 *
 * This used to null the token columns and keep the row, on the reasoning that
 * the row was doing two jobs — the OAuth grant, and how somebody who signed in
 * with that provider gets back in — so deleting it would take the second away
 * to undo the first.
 *
 * That reasoning is wrong on this origin, and the bug it caused is worse than
 * the one it was avoiding. **Nobody can sign in with a provider here at all**:
 * the only door is an email address and a code, `emailAndPassword` is off, and
 * the provider sign-in routes are disabled. The row is purely a grant.
 *
 * What keeping it cost: Better Auth keys `account` on `(issuer, accountId)` and
 * refuses to link an identity that already belongs to another user. A cleared
 * row is still that binding, so disconnecting Google from one Guaca account
 * permanently blocked authorizing that same Google on another one — and because
 * a scopeless row is not a connection, no card was left to disconnect a second
 * time. The failure was `account_already_linked_to_different_user`, and there
 * was no way out of it from inside the product.
 *
 * So the row goes. The grant has already been revoked at the provider by the
 * time this is called; what is left is a tombstone, and a tombstone that blocks
 * a future authorization is not a safety net.
 */
export async function forgetGrant(env: Env, accountRowId: string): Promise<void> {
  const db = drizzle(env.DB, { schema });
  await db.delete(account).where(eq(account.id, accountRowId));
}
