import { applyD1Migrations, env, fetchMock, SELF } from "cloudflare:test";
import { beforeAll, describe, expect, it } from "vitest";

declare module "cloudflare:test" {
  interface ProvidedEnv extends Env { MIGRATIONS: D1Migration[] }
}

const origin = "http://localhost:8787";
const bearer = "test-bearer-that-is-long-enough-for-testing";

beforeAll(async () => {
  await applyD1Migrations(env.DB, env.MIGRATIONS);
  await env.DB.exec("CREATE TABLE handoff (hash TEXT PRIMARY KEY, expires_at INTEGER NOT NULL)");
  await env.DB.exec("INSERT INTO user (id, name, email, emailVerified, createdAt, updatedAt) VALUES ('selfhost-operator', 'Operator', 'operator@localhost.invalid', 1, unixepoch(), unixepoch())");
});

describe("self-hosted handoff", () => {
  it("requires the bearer and creates one session through the real handler", async () => {
    const endpoint = `${origin}/api/selfhost/handoff`;
    expect((await SELF.fetch(endpoint, { method: "POST", headers: { authorization: "Bearer wrong" } })).status).toBe(401);
    const issued = await SELF.fetch(endpoint, { method: "POST", headers: { authorization: `Bearer ${bearer}` } });
    expect(issued.status).toBe(200);
    const { code } = await issued.json() as { code: string };
    expect(code).toMatch(/^[a-f0-9]{64}$/);
    const row = await env.DB.prepare("SELECT hash FROM handoff").first<{ hash: string }>();
    expect(row?.hash).not.toBe(code);
    const entered = await SELF.fetch(`${origin}/enter?code=${code}`, { redirect: "manual" });
    expect(entered.status).toBe(303);
    expect(entered.headers.get("referrer-policy")).toBe("no-referrer");
    const cookie = entered.headers.getSetCookie().map((value) => value.split(";")[0]).join("; ");
    const page = await SELF.fetch(`${origin}/app`, { headers: { cookie }, redirect: "manual" });
    expect(page.status).toBe(200);
    expect((await SELF.fetch(`${origin}/enter?code=${code}`, { redirect: "manual" })).status).toBe(401);
    expect((await SELF.fetch(`${origin}/signin`, { redirect: "manual" })).status).toBe(404);
  });

  it("rejects expired codes and consumes a code only once under concurrency", async () => {
    const issue = async () => {
      const response = await SELF.fetch(`${origin}/api/selfhost/handoff`, { method: "POST", headers: { authorization: `Bearer ${bearer}` } });
      return (await response.json() as { code: string }).code;
    };
    const expired = await issue();
    await env.DB.prepare("UPDATE handoff SET expires_at = 0").run();
    expect((await SELF.fetch(`${origin}/enter?code=${expired}`, { redirect: "manual" })).status).toBe(401);
    const code = await issue();
    const responses = await Promise.all(Array.from({ length: 2 }, () => SELF.fetch(`${origin}/enter?code=${code}`, { redirect: "manual" })));
    expect(responses.map((r) => r.status).sort()).toEqual([303, 401]);
  });

  it("links Google and calls a Google tool with a fake provider", async () => {
    const issued = await SELF.fetch(`${origin}/api/selfhost/handoff`, { method: "POST", headers: { authorization: `Bearer ${bearer}` } });
    const { code } = await issued.json() as { code: string };
    const entered = await SELF.fetch(`${origin}/enter?code=${code}`, { redirect: "manual" });
    const session = entered.headers.getSetCookie().map((value) => value.split(";")[0]).join("; ");
    const started = await SELF.fetch(`${origin}/app/authorize`, {
      method: "POST", redirect: "manual",
      headers: { origin, cookie: session },
      body: new URLSearchParams({ provider: "google", capability: "calendar" }),
    });
    expect(started.status).toBe(302);
    const authorization = new URL(started.headers.get("location") ?? "");
    expect(authorization.origin).toBe("https://accounts.google.com");
    const oauthCookie = started.headers.getSetCookie().map((value) => value.split(";")[0]).join("; ");
    const encoded = (value: unknown) => btoa(JSON.stringify(value)).replace(/=/g, "").replace(/\+/g, "-").replace(/\//g, "_");
    const idToken = `${encoded({ alg: "none" })}.${encoded({ sub: "google-user", email: "operator@example.com", email_verified: true, name: "Operator" })}.signature`;
    fetchMock.activate();
    fetchMock.disableNetConnect();
    try {
      fetchMock.get("https://oauth2.googleapis.com").intercept({ path: "/token", method: "POST" }).reply(200, {
        access_token: "google-access", refresh_token: "google-refresh", token_type: "Bearer", expires_in: 1,
        scope: "openid email profile https://www.googleapis.com/auth/calendar", id_token: idToken,
      }, { headers: { "content-type": "application/json" } });
      const callback = await SELF.fetch(`${origin}/api/auth/callback/google?code=fake-code&state=${encodeURIComponent(authorization.searchParams.get("state") ?? "")}`, {
        headers: { cookie: `${session}; ${oauthCookie}` }, redirect: "manual",
      });
      expect(callback.headers.get("location")).toContain("state=authorized");
      const connectors = await SELF.fetch(`${origin}/api/connectors`, { headers: { authorization: `Bearer ${bearer}` } });
      const listed = await connectors.json() as { connections: { capabilities: string[] }[] };
      expect(listed.connections[0]?.capabilities).toContain("calendar");
      fetchMock.get("https://oauth2.googleapis.com").intercept({ path: "/token", method: "POST" }).reply(200, {
        access_token: "google-refreshed", token_type: "Bearer", expires_in: 3600,
        scope: "openid email profile https://www.googleapis.com/auth/calendar",
      }, { headers: { "content-type": "application/json" } });
      fetchMock.get("https://www.googleapis.com").intercept({ path: /\/calendar\/v3\/calendars\/primary\/events\?/, method: "GET" }).reply(200, { items: [] }, { headers: { "content-type": "application/json" } });
      const call = await SELF.fetch(`${origin}/mcp`, {
        method: "POST", headers: { authorization: `Bearer ${bearer}`, "content-type": "application/json" },
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "calendar_list_events", arguments: {} } }),
      });
      expect(call.status).toBe(200);
      expect(await call.text()).not.toContain('"isError":true');
      fetchMock.assertNoPendingInterceptors();
    } finally {
      fetchMock.deactivate();
    }
  });
});
