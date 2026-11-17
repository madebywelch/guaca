import { applyD1Migrations, env } from "cloudflare:test";
import { beforeAll, describe, expect, it } from "vitest";

import { getAuth } from "./auth";
import { readConnections } from "./connections";
import { configuredProviders } from "./connectors";

declare module "cloudflare:test" {
  interface ProvidedEnv extends Env {
    MIGRATIONS: D1Migration[];
    PREVIOUS_DB: D1Database;
  }
}

const LABEL_MIGRATIONS = ["0005_account_label.sql", "0006_account_label_from_id_token.sql"];
const BACKFILL = LABEL_MIGRATIONS[1] as string;

const base64url = (value: string) =>
  btoa(String.fromCharCode(...new TextEncoder().encode(value)))
    .replace(/=+$/, "")
    .replace(/\+/g, "-")
    .replace(/\//g, "_");

const jwt = (claims: Record<string, unknown>) =>
  `${base64url(JSON.stringify({ alg: "RS256", kid: "test-key", typ: "JWT" }))}.${base64url(JSON.stringify(claims))}.c2lnbmF0dXJl`;

const googleClaims = (sub: string, email: string, name: string) => ({
  iss: "https://accounts.google.com",
  azp: "test-google-client",
  aud: "test-google-client",
  sub,
  email,
  email_verified: true,
  at_hash: "HK6E_P6Dh8Y93mRNtsDB1Q",
  name,
  picture: `https://lh3.googleusercontent.com/a/${sub}=s96-c`,
  given_name: name.split(" ")[0],
  family_name: name.split(" ")[1] ?? "",
  iat: 1763380000,
  exp: 1763383600,
});

function previousLabel(idToken: string | null, accountId: string): string {
  if (!idToken) return accountId;
  try {
    const [, payload] = idToken.split(".");
    if (!payload) return accountId;
    const claims = JSON.parse(atob(payload.replace(/-/g, "+").replace(/_/g, "/"))) as {
      email?: unknown;
    };
    return typeof claims.email === "string" && claims.email ? claims.email : accountId;
  } catch {
    return accountId;
  }
}

interface Row {
  id: string;
  providerId: string;
  accountId: string;
  idToken: string | null;
}

const ROWS: Row[] = [
  { id: "a1", providerId: "google", accountId: "104812345678901234567", idToken: jwt(googleClaims("104812345678901234567", "ada@gmail.com", "Ada Lovelace")) },
  { id: "a2", providerId: "google", accountId: "104812345678901234568", idToken: jwt(googleClaims("104812345678901234568", "grace.hopper@example.org", "Grace Hopper")) },
  { id: "a3", providerId: "google", accountId: "104812345678901234569", idToken: jwt(googleClaims("104812345678901234569", "zoe+work@example.co", "Zoë Ångström")) },
  { id: "a4", providerId: "google", accountId: "104812345678901234570", idToken: jwt({ iss: "https://accounts.google.com", sub: "104812345678901234570" }) },
  { id: "a5", providerId: "google", accountId: "104812345678901234571", idToken: `${base64url("{}")}.${base64url("not json")}.sig` },
  { id: "a6", providerId: "google", accountId: "104812345678901234572", idToken: "not-a-jwt" },
  { id: "a7", providerId: "google", accountId: "104812345678901234573", idToken: null },
];

async function previousSchema(db: D1Database) {
  const all = env.MIGRATIONS;
  await applyD1Migrations(
    db,
    all.filter((migration) => !LABEL_MIGRATIONS.includes(migration.name)),
  );
  await db
    .prepare(
      "INSERT INTO user (id, name, email, emailVerified, createdAt, updatedAt) VALUES ('owner', 'Owner', 'owner@example.com', 1, unixepoch(), unixepoch())",
    )
    .run();
  await db.batch(
    ROWS.map((row) =>
      db
        .prepare(
          "INSERT INTO account (id, issuer, accountId, providerId, userId, accessToken, refreshToken, idToken, scope, createdAt, updatedAt) VALUES (?, 'https://accounts.google.com', ?, ?, 'owner', 'access', 'refresh', ?, 'openid email https://www.googleapis.com/auth/calendar', unixepoch(), unixepoch())",
        )
        .bind(row.id, row.accountId, row.providerId, row.idToken),
    ),
  );
}

const migration = (name: string) => {
  const found = env.MIGRATIONS.find((candidate) => candidate.name === name);
  if (!found) throw new Error(`no migration named ${name}`);
  return found;
};

async function snapshot(db: D1Database) {
  const { results } = await db
    .prepare("SELECT id, accountId, idToken, label, updatedAt FROM account ORDER BY id")
    .all<{ id: string; accountId: string; idToken: string | null; label: string | null; updatedAt: number }>();
  return results;
}

describe("the migration that replaces the identity token with a label", () => {
  const db = () => env.PREVIOUS_DB;

  beforeAll(async () => {
    await previousSchema(db());
    await applyD1Migrations(db(), LABEL_MIGRATIONS.map(migration));
  });

  it("keeps every connection's label identical to what it was before", async () => {
    const providers = configuredProviders(env as unknown as Record<string, unknown>);
    const connections = await readConnections({ ...env, DB: db() }, "owner", providers);
    const labels = Object.fromEntries(connections.map((connection) => [connection.id, connection.label]));
    expect(labels).toEqual(
      Object.fromEntries(ROWS.map((row) => [row.id, previousLabel(row.idToken, row.accountId)])),
    );
    expect(labels.a1).toBe("ada@gmail.com");
    expect(labels.a3).toBe("zoe+work@example.co");
    expect(labels.a4).toBe("104812345678901234570");
  });

  it("leaves no identity token on any row", async () => {
    const rows = await snapshot(db());
    expect(rows).toHaveLength(ROWS.length);
    expect(rows.filter((row) => row.idToken !== null)).toEqual([]);
  });

  it("changes nothing when it runs a second time", async () => {
    const before = await snapshot(db());
    for (const query of migration(BACKFILL).queries) await db().prepare(query).run();
    expect(await snapshot(db())).toEqual(before);
  });

  it("fills a label only where it is empty and a token is present", async () => {
    await db()
      .prepare("UPDATE account SET label = 'kept@example.com', idToken = ? WHERE id = 'a1'")
      .bind(jwt(googleClaims("104812345678901234567", "other@example.com", "Ada Lovelace")))
      .run();
    await db()
      .prepare("UPDATE account SET idToken = ? WHERE id = 'a4'")
      .bind(jwt(googleClaims("104812345678901234570", "late@example.com", "Late Link")))
      .run();
    for (const query of migration(BACKFILL).queries) await db().prepare(query).run();
    const rows = Object.fromEntries((await snapshot(db())).map((row) => [row.id, row]));
    expect(rows.a1?.label).toBe("kept@example.com");
    expect(rows.a4?.label).toBe("late@example.com");
    expect(rows.a7?.label).toBeNull();
    expect(Object.values(rows).filter((row) => row.idToken !== null)).toEqual([]);
  });
});

describe("an account Better Auth writes", () => {
  let context: Awaited<ReturnType<typeof getAuth>["$context"]>;

  beforeAll(async () => {
    await applyD1Migrations(env.DB, env.MIGRATIONS);
    await env.DB.prepare(
      "INSERT INTO user (id, name, email, emailVerified, createdAt, updatedAt) VALUES ('writer', 'Writer', 'writer@example.com', 1, unixepoch(), unixepoch())",
    ).run();
    context = await getAuth(env).$context;
  });

  const stored = (id: string) =>
    env.DB.prepare("SELECT idToken, label FROM account WHERE id = ?")
      .bind(id)
      .first<{ idToken: string | null; label: string | null }>();

  it("is created with the email as its label and no identity token", async () => {
    const created = await context.internalAdapter.createAccount({
      userId: "writer",
      issuer: "https://accounts.google.com",
      providerId: "google",
      accountId: "created-sub",
      accessToken: "access",
      idToken: jwt(googleClaims("created-sub", "created@example.com", "Created Person")),
    });
    expect(await stored(created.id)).toEqual({ idToken: null, label: "created@example.com" });
  });

  it("is updated with the new email as its label and no identity token", async () => {
    const created = await context.internalAdapter.createAccount({
      userId: "writer",
      issuer: "https://accounts.google.com",
      providerId: "google",
      accountId: "updated-sub",
      accessToken: "access",
      idToken: jwt(googleClaims("updated-sub", "before@example.com", "Updated Person")),
    });
    await context.internalAdapter.updateAccount(created.id, {
      accessToken: "refreshed",
      idToken: jwt(googleClaims("updated-sub", "after@example.com", "Updated Person")),
    });
    expect(await stored(created.id)).toEqual({ idToken: null, label: "after@example.com" });
  });

  it("keeps its label when an update carries no identity token", async () => {
    const created = await context.internalAdapter.createAccount({
      userId: "writer",
      issuer: "https://accounts.google.com",
      providerId: "google",
      accountId: "kept-sub",
      accessToken: "access",
      idToken: jwt(googleClaims("kept-sub", "kept@example.com", "Kept Person")),
    });
    await context.internalAdapter.updateAccount(created.id, { accessToken: "refreshed" });
    expect(await stored(created.id)).toEqual({ idToken: null, label: "kept@example.com" });
  });

  it("is created with no label when the provider issues no identity token", async () => {
    const created = await context.internalAdapter.createAccount({
      userId: "writer",
      issuer: "https://github.com",
      providerId: "github",
      accountId: "4242",
      accessToken: "access",
    });
    expect(await stored(created.id)).toEqual({ idToken: null, label: null });
  });
});
