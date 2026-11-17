import {
  oauthProviderAuthServerMetadata,
  oauthProviderOpenIdConfigMetadata,
} from "@better-auth/oauth-provider";
import { type Context, Hono } from "hono";
import { deleteCookie, getSignedCookie, setSignedCookie } from "hono/cookie";
import { csrf } from "hono/csrf";
import { HTTPException } from "hono/http-exception";
import type { ReactNode } from "react";

import { renderDocument } from "../src/account/document";
import * as pages from "../src/account/pages";
import { type Auth, CONNECTORS_SCOPE, getAuth, MCP_SCOPES, SERVED_AUTH_PATHS } from "./auth";
import { codeAlreadySent } from "./codes";
import { readConnection, readConnections } from "./connections";
import {
  configuredProviders,
  findProvider,
  narrows,
  PROVIDERS,
  parseScopes,
  revokeGrant,
  scopesFor,
} from "./connectors";
import { CODE_TTL_MINUTES } from "./email";
import { forgetGrant } from "./grants";
import * as mcp from "./mcp";
import { consumeHandoff, createHandoff, matchesBearer, SELFHOST_USER } from "./selfhost";
import { stylesheetHref } from "./stylesheet";

type AppEnv = { Bindings: Env; Variables: { auth: Auth } };
type Ctx = Context<AppEnv>;

const app = new Hono<AppEnv>({ strict: false });

const LOOPBACK = "http://127.0.0.1:* http://[::1]:*";

const CSP = [
  "default-src 'none'",
  "img-src 'self'",
  "style-src 'self' 'unsafe-inline'",
  "font-src 'self'",
  "base-uri 'none'",
  `form-action 'self' ${LOOPBACK} ${PROVIDERS.map((p) => p.consentOrigin).join(" ")}`,
  "frame-ancestors 'none'",
].join("; ");

app.use("*", async (c, next) => {
  await next();
  c.res.headers.set("Content-Security-Policy", CSP);
  c.res.headers.set("X-Content-Type-Options", "nosniff");
  c.res.headers.set("Referrer-Policy", c.req.path === "/enter" ? "no-referrer" : "strict-origin-when-cross-origin");
  if (!c.res.headers.has("Cache-Control")) {
    c.res.headers.set("Cache-Control", "private, no-store");
  }
});

const formCsrf = csrf({
  origin: (origin, c) => origin === (c.env.SELFHOST_BEARER ? c.env.BETTER_AUTH_URL : new URL(c.req.url).origin),
});
app.use("/signin/*", formCsrf);
app.use("/signout", formCsrf);
app.use("/app/*", formCsrf);
app.use("/consent", formCsrf);

app.use("/api/connectors/*", requireBearer);
app.use("/api/connectors", requireBearer);
app.use("/mcp", requireBearer);
app.use("/mcp/*", requireBearer);

app.use("*", async (c, next) => {
  c.set("auth", getAuth(c.env));
  await next();
});

async function formOf(c: Ctx): Promise<FormData> {
  try {
    return await c.req.formData();
  } catch {
    return new FormData();
  }
}

function unauthorized(c: Ctx, error?: "invalid_token") {
  const resource = protectedResourceMetadataUrl(c.env);
  const challenge = error ? `Bearer error="${error}", ` : "Bearer ";
  return c.json({ error: error ?? "unauthorized" }, 401, {
    "WWW-Authenticate": `${challenge}resource_metadata="${resource}"`,
  });
}

async function requireBearer(c: Ctx, next: () => Promise<void>) {
  if (!c.req.header("authorization")?.startsWith("Bearer ")) {
    return unauthorized(c);
  }
  await next();
}

interface Linked {
  userId: string;
  email: string;
  scopes: Set<string>;
}

async function linked(c: Ctx): Promise<Linked | null> {
  if (c.env.SELFHOST_BEARER) {
    if (!(await matchesBearer(c.req.header("Authorization"), c.env.SELFHOST_BEARER))) return null;
    return { userId: SELFHOST_USER, email: "", scopes: new Set([CONNECTORS_SCOPE]) };
  }
  try {
    const info = (await c.var.auth.api.oauth2UserInfo({
      headers: c.req.raw.headers,
    })) as { sub?: string; email?: string; scope?: string } | null;
    if (!info?.sub) return null;
    return {
      userId: info.sub,
      email: info.email ?? "",
      scopes: new Set((info.scope ?? "").split(" ").filter(Boolean)),
    };
  } catch {
    return null;
  }
}

async function page(c: { env: Env }, body: ReactNode, title: string, nav?: ReactNode) {
  const rendered = renderDocument({
    title,
    nav,
    stylesheet: await stylesheetHref(c.env),
    children: body,
  });
  return new Response(rendered, {
    headers: { "content-type": "text/html; charset=utf-8" },
  });
}

function safeNext(value: string | undefined, fallback = "/app"): string {
  if (!value?.startsWith("/") || value.startsWith("//")) return fallback;
  return value;
}

function redirectWith(headers: Headers, url: string | undefined, fallback: string): Response {
  const out = new Response(null, { status: 302, headers });
  out.headers.set("Location", url ?? fallback);
  return out;
}

const PUBLIC_CLIENT_AUTH_METHOD = "none";

const UNSUPPORTED_PROMPT = "select_account";

async function withPublicClientCorrections(res: Response): Promise<Response> {
  if (res.status !== 200) return res;
  const body = (await res.json()) as Record<string, unknown>;

  const methods = body.token_endpoint_auth_methods_supported;
  if (Array.isArray(methods) && !methods.includes(PUBLIC_CLIENT_AUTH_METHOD)) {
    body.token_endpoint_auth_methods_supported = [PUBLIC_CLIENT_AUTH_METHOD, ...methods];
  }

  const revocationMethods = body.revocation_endpoint_auth_methods_supported;
  if (Array.isArray(revocationMethods) && !revocationMethods.includes(PUBLIC_CLIENT_AUTH_METHOD)) {
    body.revocation_endpoint_auth_methods_supported = [
      PUBLIC_CLIENT_AUTH_METHOD,
      ...revocationMethods,
    ];
  }

  delete body.id_token_signing_alg_values_supported;

  const prompts = body.prompt_values_supported;
  if (Array.isArray(prompts)) {
    body.prompt_values_supported = prompts.filter((prompt) => prompt !== UNSUPPORTED_PROMPT);
  }

  delete body.end_session_endpoint;

  const headers = new Headers(res.headers);
  headers.delete("content-length");
  return new Response(JSON.stringify(body), { status: res.status, headers });
}

app.get("/api/auth/.well-known/oauth-authorization-server", async (c) =>
  withPublicClientCorrections(await c.var.auth.handler(c.req.raw)),
);

app.get("/api/auth/.well-known/openid-configuration", async (c) =>
  withPublicClientCorrections(await c.var.auth.handler(c.req.raw)),
);

for (const path of SERVED_AUTH_PATHS) {
  app.all(`/api/auth${path}`, (c) => {
    if (c.env.SELFHOST_BEARER && c.req.path !== "/api/auth/callback/google") {
      return c.notFound();
    }
    return c.var.auth.handler(c.req.raw);
  });
}

app.all("/api/auth/*", (c) => c.notFound());

app.post("/api/selfhost/handoff", async (c) => {
  if (!c.env.SELFHOST_BEARER || !(await matchesBearer(c.req.header("Authorization"), c.env.SELFHOST_BEARER))) {
    return c.json({ error: "unauthorized" }, 401);
  }
  return c.json({ code: await createHandoff(c.env.DB) });
});

app.get("/enter", async (c) => {
  if (!c.env.SELFHOST_BEARER || !(await consumeHandoff(c.env.DB, c.req.query("code")))) {
    return c.text("This link has expired or was already used.", 401);
  }
  const context = await c.var.auth.$context;
  const session = await context.internalAdapter.createSession(SELFHOST_USER);
  if (!session) return c.text("Could not start a session.", 500);
  const cookie = context.authCookies.sessionToken;
  await setSignedCookie(c, cookie.name, session.token, context.secret, cookie.attributes);
  c.header("Referrer-Policy", "no-referrer");
  return c.redirect("/app", 303);
});

app.get("/.well-known/oauth-authorization-server", async (c) =>
  withPublicClientCorrections(await oauthProviderAuthServerMetadata(c.var.auth)(c.req.raw)),
);

app.get("/.well-known/oauth-authorization-server/api/auth", async (c) =>
  withPublicClientCorrections(await oauthProviderAuthServerMetadata(c.var.auth)(c.req.raw)),
);

const openIdConfiguration = async (c: Ctx) =>
  withPublicClientCorrections(await oauthProviderOpenIdConfigMetadata(c.var.auth)(c.req.raw));

app.get("/.well-known/openid-configuration", openIdConfiguration);
app.get("/.well-known/openid-configuration/api/auth", openIdConfiguration);

function protectedResourceMetadata(c: Ctx) {
  const origin = new URL(c.env.BETTER_AUTH_URL).origin;
  return c.json({
    resource: mcpResource(c.env),
    authorization_servers: [`${origin}/api/auth`],
    scopes_supported: [...MCP_SCOPES],
    bearer_methods_supported: ["header"],
  });
}

function mcpResource(env: Env): string {
  return `${new URL(env.BETTER_AUTH_URL).origin}/mcp`;
}

function protectedResourceMetadataUrl(env: Env): string {
  const resource = new URL(mcpResource(env));
  return `${resource.origin}/.well-known/oauth-protected-resource${resource.pathname}`;
}

app.get("/.well-known/oauth-protected-resource/mcp", protectedResourceMetadata);

app.get("/.well-known/oauth-protected-resource", protectedResourceMetadata);

const PENDING = "guaca_signin_email";

const errors: Record<string, string> = {
  required: "Sign in to continue.",
  send: "That code could not be sent. Check the address and try again.",
  code: "That code is not right, or it has expired. Ask for another.",
  expired: "That code has expired. Ask for another.",
};

function pendingCookie(env: Env) {
  return {
    path: "/signin",
    httpOnly: true,
    secure: env.BETTER_AUTH_URL.startsWith("https://"),
    sameSite: "Lax" as const,
    maxAge: CODE_TTL_MINUTES * 60,
  };
}

function resumeAuthorization(c: Ctx): string | undefined {
  const url = new URL(c.req.url);
  if (!url.searchParams.has("sig")) return undefined;
  return `/api/auth/oauth2/authorize${url.search}`;
}

app.get("/signin", async (c) => {
  if (c.env.SELFHOST_BEARER) return c.notFound();
  const next = resumeAuthorization(c) ?? safeNext(c.req.query("next"));
  const session = await c.var.auth.api.getSession({ headers: c.req.raw.headers });
  if (session) return c.redirect(next, 302);

  const body = pages.signIn({
    next,
    error: errors[c.req.query("error") ?? ""],
  });
  return page(c, body, "Sign in to Guaca");
});

app.post("/signin/email", async (c) => {
  if (c.env.SELFHOST_BEARER) return c.notFound();
  const form = await formOf(c);
  const email = String(form.get("email") ?? "")
    .trim()
    .toLowerCase();
  const next = safeNext(form.get("next")?.toString());

  if (await codeAlreadySent(c.env, email)) {
    console.log("sign-in code asked for twice; the first one still stands");
  } else {
    try {
      await c.var.auth.api.sendVerificationOTP({
        body: { email, type: "sign-in" },
        headers: c.req.raw.headers,
      });
    } catch (error) {
      console.error("sign-in code not sent", { error });
      return c.redirect("/signin?error=send", 302);
    }
  }

  await setSignedCookie(c, PENDING, email, c.env.BETTER_AUTH_SECRET, pendingCookie(c.env));
  return c.redirect(`/signin/code?next=${encodeURIComponent(next)}`, 302);
});

app.get("/signin/code", async (c) => {
  if (c.env.SELFHOST_BEARER) return c.notFound();
  const session = await c.var.auth.api.getSession({ headers: c.req.raw.headers });
  if (session) return c.redirect(safeNext(c.req.query("next")), 302);

  const email = await getSignedCookie(c, c.env.BETTER_AUTH_SECRET, PENDING);
  if (!email) return c.redirect("/signin", 302);

  const body = pages.signInCode({
    email,
    next: safeNext(c.req.query("next")),
    error: errors[c.req.query("error") ?? ""],
  });
  return page(c, body, "Enter your sign-in code");
});

app.post("/signin/code", async (c) => {
  if (c.env.SELFHOST_BEARER) return c.notFound();
  const email = await getSignedCookie(c, c.env.BETTER_AUTH_SECRET, PENDING);
  if (!email) return c.redirect("/signin?error=expired", 302);

  const form = await formOf(c);
  const otp = String(form.get("code") ?? "").trim();
  const next = safeNext(form.get("next")?.toString());

  let headers: Headers;
  try {
    ({ headers } = await c.var.auth.api.signInEmailOTP({
      body: { email, otp },
      headers: c.req.raw.headers,
      returnHeaders: true,
    }));
  } catch {
    return c.redirect(`/signin/code?error=code&next=${encodeURIComponent(next)}`, 302);
  }

  deleteCookie(c, PENDING, pendingCookie(c.env));
  return redirectWith(headers, next, next);
});

app.post("/signout", async (c) => {
  const { headers } = await c.var.auth.api.signOut({
    headers: c.req.raw.headers,
    returnHeaders: true,
  });
  return redirectWith(headers, "/", "/");
});

app.get("/app", async (c) => {
  const session = await c.var.auth.api.getSession({ headers: c.req.raw.headers });
  if (!session) return c.redirect("/signin?error=required&next=%2Fapp", 302);

  const providers = configuredProviders(c.env as unknown as Record<string, unknown>);
  const connections = await readConnections(c.env, session.user.id, providers);

  const notices: Record<string, { text: string; bad?: boolean }> = {
    authorized: { text: "Authorized. Your crews can now choose this account under Plugins." },
    disconnected: { text: "Revoked at the provider and removed here." },
    failed: { text: "That did not complete. Nothing was changed.", bad: true },
    nothing: {
      text: "Nothing was ticked, so there was nothing to authorize. Tick what your agents may reach first.",
      bad: true,
    },
    "revoke-failed": {
      text: "The token could not be revoked at the provider, so nothing was removed here either. Try again, or remove the grant from your account settings at the provider.",
      bad: true,
    },
  };

  const failures: Record<string, { text: string; bad: boolean }> = {
    account_already_linked_to_different_user: {
      text: "That Google account is already attached to a different Guaca account, so it cannot be authorized here as well. Sign in as that account and disconnect it there first, then come back and authorize it.",
      bad: true,
    },
    account_not_linked: {
      text: "That account could not be attached to this one. Sign in again and retry.",
      bad: true,
    },
  };

  const failed = c.req.query("error");
  const notice =
    (failed ? failures[failed] : undefined) ??
    (failed
      ? {
          text: `That did not complete, and the provider gave the reason as "${failed}". Nothing was changed.`,
          bad: true,
        }
      : undefined) ??
    notices[c.req.query("state") ?? ""];

  const body = pages.connectors({
    providers,
    selfHosted: Boolean(c.env.SELFHOST_BEARER),
    connections: connections.map((connection) => ({
      id: connection.id,
      providerId: connection.providerId,
      label: connection.label,
      capabilities: connection.capabilities,
    })),
    notice,
  });
  return page(c, body, "Connectors", pages.signedInNav(c.env.SELFHOST_BEARER ? "Your connector" : session.user.email));
});

app.post("/app/authorize", async (c) => {
  const session = await c.var.auth.api.getSession({ headers: c.req.raw.headers });
  if (!session) return c.redirect("/signin?error=required&next=%2Fapp", 302);

  const form = await formOf(c);
  const provider = findProvider(String(form.get("provider") ?? ""));
  if (!provider) return c.redirect("/app?state=failed", 302);

  const desired = new Set(form.getAll("capability").map(String));

  const connection = form.get("connection")
    ? await readConnection(
        c.env,
        session.user.id,
        [provider],
        String(form.get("connection")),
      )
    : undefined;
  if (form.get("connection") && !connection) return c.redirect("/app?state=failed", 302);

  const held = parseScopes(connection?.scope);
  const granted = new Set(
    provider.capabilities
      .filter((capability) => capability.scopes.some((scope) => held.has(scope)))
      .map((capability) => capability.id),
  );

  if (desired.size === 0) {
    if (!connection) return c.redirect("/app?state=nothing", 302);

    return disconnect(c, provider.id, connection?.id);
  }

  if (connection && narrows(granted, desired)) {
    const revoked = await revokeIfPossible(c, provider.id, connection.id);
    if (!revoked) return c.redirect("/app?state=revoke-failed", 302);
    await forgetGrant(c.env, connection.id);
  }

  const { headers, response } = await c.var.auth.api.linkSocialAccount({
    body: {
      provider: provider.id,
      scopes: scopesFor(provider, desired),
      callbackURL: "/app?state=authorized",
      errorCallbackURL: "/app?state=failed",
      ...(provider.id === "google"
        ? {
            additionalParams: { access_type: "offline", prompt: "select_account consent" },
          }
        : {}),
    },
    headers: c.req.raw.headers,
    returnHeaders: true,
  });
  return redirectWith(headers, response.url, "/app?state=failed");
});

app.post("/app/disconnect", async (c) => {
  const session = await c.var.auth.api.getSession({ headers: c.req.raw.headers });
  if (!session) return c.redirect("/signin?error=required&next=%2Fapp", 302);

  const form = await formOf(c);
  const provider = findProvider(String(form.get("provider") ?? ""));
  if (!provider) return c.redirect("/app?state=failed", 302);

  const connection = await readConnection(
    c.env,
    session.user.id,
    [provider],
    String(form.get("connection") ?? ""),
  );
  if (!connection) return c.redirect("/app?state=failed", 302);

  return disconnect(c, provider.id, connection.id);
});

app.get("/consent", async (c) => {
  const query = new URL(c.req.url).search.slice(1);
  if (!query) return c.redirect("/app", 302);

  const session = await c.var.auth.api.getSession({ headers: c.req.raw.headers });
  if (!session) {
    const next = `/consent?${query}`;
    return c.redirect(`/signin?error=required&next=${encodeURIComponent(next)}`, 302);
  }

  const asked = new URLSearchParams(query);
  return page(
    c,
    pages.consent({
      email: session.user.email,
      scopes: describeScopes(asked.get("scope")),
      query,
    }),
    "Link Guaca to your account",
    pages.signedInNav(session.user.email),
  );
});

app.post("/consent", async (c) => {
  const session = await c.var.auth.api.getSession({ headers: c.req.raw.headers });
  if (!session) return c.redirect("/signin?error=required&next=%2Fapp", 302);

  const form = await formOf(c);
  const query = String(form.get("oauth_query") ?? "");
  const accept = form.get("decision") === "allow";

  let redirect: string | undefined;
  try {
    const answered = await c.var.auth.api.oauth2Consent({
      body: { accept, oauth_query: query },
      headers: c.req.raw.headers,
      request: new Request(c.req.url, { method: "POST", headers: c.req.raw.headers }),
    });

    const payload = (answered instanceof Response ? await answered.json() : answered) as {
      url?: string;
      redirect_uri?: string;
    } | null;
    redirect = payload?.url ?? payload?.redirect_uri;
  } catch {
    return page(
      c,
      pages.notice({
        heading: "That request expired.",
        body: "Start the sign-in again from Guaca. Nothing was linked.",
        bad: true,
        back: "/app",
      }),
      "Link Guaca to your account",
      pages.signedInNav(session.user.email),
    );
  }

  if (!redirect) return c.redirect("/app?state=failed", 302);
  return c.redirect(redirect, 302);
});

const SCOPE_COPY: Record<string, string> = {
  openid: "Know which account it is linked to",
  email: "See the address this account signs in with",
  offline_access: "Stay linked without asking again every hour",
  connectors: "Use the connectors you have authorized, and get access tokens for them",
};

function describeScopes(scope: string | null): string[] {
  const asked = (scope ?? "").split(" ").filter(Boolean);
  if (asked.length === 0) return [];
  return asked.map((name) => SCOPE_COPY[name] ?? name);
}

app.post("/mcp", async (c) => serveMcp(c, undefined));

app.post("/mcp/:connection", async (c) => serveMcp(c, c.req.param("connection")));

async function serveMcp(c: Ctx, connection: string | undefined) {
  const who = await linked(c);
  if (!who) return unauthorized(c, "invalid_token");
  if (!who.scopes.has(CONNECTORS_SCOPE)) return c.json({ error: "insufficient_scope" }, 403);
  return mcp.handle(c.req.raw, c.env, c.var.auth, {
    userId: who.userId,
    connection,
  });
}

app.get("/api/connectors", async (c) => {
  const who = await linked(c);
  if (!who) return unauthorized(c, "invalid_token");
  if (!who.scopes.has(CONNECTORS_SCOPE)) return c.json({ error: "insufficient_scope" }, 403);

  const providers = configuredProviders(c.env as unknown as Record<string, unknown>);
  const connections = await readConnections(c.env, who.userId, providers);

  return c.json({
    user: { email: who.email },
    connections: connections.map((connection) => ({
      id: connection.id,
      provider: connection.providerId,
      label: connection.label,
      capabilities: [...connection.capabilities],
    })),
    providers: providers.map((provider) => ({
      id: provider.id,
      label: provider.label,
      capabilities: provider.capabilities.map((capability) => ({
        id: capability.id,
        label: capability.label,
        granted: connections.some(
          (connection) =>
            connection.providerId === provider.id && connection.capabilities.has(capability.id),
        ),
      })),
    })),
  });
});

app.post("/api/connectors/:provider/token", async (c) => {
  const who = await linked(c);
  if (!who) return unauthorized(c, "invalid_token");
  if (!who.scopes.has(CONNECTORS_SCOPE)) return c.json({ error: "insufficient_scope" }, 403);

  const provider = findProvider(c.req.param("provider"));
  if (!provider) return c.json({ error: "unknown_provider" }, 404);

  const all = await readConnections(c.env, who.userId, [provider]);
  const asked = c.req.query("connection");
  const grant = asked
    ? all.find((connection) => connection.id === asked)
    : all.find((connection) => connection.capabilities.size > 0);
  if (!grant || grant.capabilities.size === 0) return c.json({ error: "not_authorized" }, 409);

  try {
    const token = await c.var.auth.api.getAccessToken({
      body: { accountId: grant.id, userId: who.userId },
    });
    return c.json({
      provider: provider.id,
      connection: grant.id,
      accessToken: token.accessToken,
      expiresAt: token.accessTokenExpiresAt ?? null,
      capabilities: [...grant.capabilities],
    });
  } catch {
    return c.json({ error: "grant_expired" }, 409);
  }
});

app.notFound(async (c) => {
  const asset = await c.env.ASSETS.fetch(c.req.raw);
  if (asset.status !== 404) return asset;
  return c.text("Not found", 404);
});

app.onError((error, c) => {
  if (error instanceof HTTPException) return error.getResponse();
  console.error("worker error", { path: new URL(c.req.url).pathname, error });
  return c.text("Something went wrong", 500);
});

async function disconnect(c: Ctx, providerId: string, accountRowId: string | undefined) {
  if (!accountRowId) return c.redirect("/app?state=disconnected", 302);
  const revoked = await revokeIfPossible(c, providerId, accountRowId);
  if (!revoked) return c.redirect("/app?state=revoke-failed", 302);
  await forgetGrant(c.env, accountRowId);
  return c.redirect("/app?state=disconnected", 302);
}

async function revokeIfPossible(
  c: Ctx,
  providerId: string,
  accountRowId: string,
): Promise<boolean> {
  let token: string | null | undefined;
  try {
    const held = await c.var.auth.api.getAccessToken({
      body: { accountId: accountRowId },
      headers: c.req.raw.headers,
    });
    token = held.accessToken;
  } catch {
    return true;
  }
  if (!token) return true;

  try {
    await revokeGrant(providerId, token, c.env as unknown as Record<string, unknown>);
    return true;
  } catch (error) {
    console.error("revoke failed", { providerId, error });
    return false;
  }
}

export default app;
