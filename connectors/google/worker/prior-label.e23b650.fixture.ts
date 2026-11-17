import { decryptOAuthToken } from "better-auth/oauth2";

type DecryptContext = Parameters<typeof decryptOAuthToken>[1];

export async function labelFor(
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
