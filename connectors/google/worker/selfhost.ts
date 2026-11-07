import { constantTimeEqual } from "better-auth/crypto";

export const SELFHOST_USER = "selfhost-operator";

async function digest(value: string): Promise<string> {
  const bytes = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
  return Array.from(new Uint8Array(bytes), (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export async function matchesBearer(header: string | undefined, expected: string): Promise<boolean> {
  const supplied = header?.startsWith("Bearer ") ? header.slice(7) : "";
  return constantTimeEqual(await digest(supplied), await digest(expected));
}

export async function createHandoff(db: D1Database): Promise<string> {
  const code = Array.from(crypto.getRandomValues(new Uint8Array(32)), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  await db.prepare("DELETE FROM handoff WHERE expires_at <= ?").bind(Date.now()).run();
  await db.prepare("INSERT INTO handoff (hash, expires_at) VALUES (?, ?)")
    .bind(await digest(code), Date.now() + 60_000).run();
  return code;
}

export async function consumeHandoff(db: D1Database, code: string | undefined): Promise<boolean> {
  if (!code || !/^[0-9a-f]{64}$/.test(code)) return false;
  const row = await db.prepare("DELETE FROM handoff WHERE hash = ? AND expires_at > ? RETURNING hash")
    .bind(await digest(code), Date.now()).first();
  return row !== null;
}
