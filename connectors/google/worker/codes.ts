import { and, eq, gt } from "drizzle-orm";
import { drizzle } from "drizzle-orm/d1";

import { schema, verification } from "./schema";

/**
 * Whether the address already has a sign-in code on its way.
 *
 * Minting a code replaces the last one. Better Auth reads the newest row for
 * an identifier and spending it deletes the whole identifier, so a second code
 * does not sit alongside the first: it kills it. That makes a duplicate post
 * of the sign-in form worse than noise. Two emails arrive, they are
 * indistinguishable apart from the digits, and the one a person opens first is
 * the one that no longer works.
 *
 * A post is a duplicate for as long as the code it would replace is still
 * useful to somebody who has not read their email yet. A minute covers the
 * double click, the impatient second submit and a browser retrying a post it
 * did not see the answer to; it is short enough that a person who genuinely
 * did not receive the first one is not made to wait for the ten-minute
 * expiry to ask again.
 *
 * The check is a read, so two posts landing inside the same round trip can
 * still both mint. That window is the length of one D1 write rather than the
 * length of a send, which is what it was before.
 */
export const RESEND_AFTER_SECONDS = 60;

/**
 * Better Auth's key for a sign-in code: `toOTPIdentifier` in the `emailOTP`
 * plugin builds `${type}-otp-${email}`.
 *
 * That is an internal shape, and this file is the only place that assumes it,
 * so `worker.itest.ts` asks the library to send a code and fails if the row it
 * writes is not the row this names.
 */
export function signInCodeIdentifier(email: string): string {
  return `sign-in-otp-${email}`;
}

/** True when a code was minted for this address inside the window. */
export async function codeAlreadySent(env: Env, email: string): Promise<boolean> {
  const since = new Date(Date.now() - RESEND_AFTER_SECONDS * 1000);
  const rows = await drizzle(env.DB, { schema })
    .select({ id: verification.id })
    .from(verification)
    .where(
      and(
        eq(verification.identifier, signInCodeIdentifier(email)),
        gt(verification.createdAt, since),
      ),
    )
    .limit(1);
  return rows.length > 0;
}
