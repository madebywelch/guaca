import { eq } from "drizzle-orm";
import { drizzle } from "drizzle-orm/d1";

import { account, schema } from "./schema";

export async function forgetGrant(env: Env, accountRowId: string): Promise<void> {
  const db = drizzle(env.DB, { schema });
  await db.delete(account).where(eq(account.id, accountRowId));
}
