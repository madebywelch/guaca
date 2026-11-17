import { and, eq, gt } from "drizzle-orm";
import { drizzle } from "drizzle-orm/d1";

import { schema, verification } from "./schema";

export const RESEND_AFTER_SECONDS = 60;

export function signInCodeIdentifier(email: string): string {
  return `sign-in-otp-${email}`;
}

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
