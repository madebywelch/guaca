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
import { type Auth, CONNECTORS_SCOPE, getAuth, MCP_SCOPES } from "./auth";
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

/**
 * Everything at guaca.bot that is not the marketing page.
 *
 * The page itself is a prerendered asset and never reaches this Worker:
 * Cloudflare serves an asset when one matches and invokes this only when none
 * does, and `run_worker_first` in `wrangler.jsonc` names the paths that must
 * be dynamic no matter what ends up in `dist/`.
 *
 * ## Why an origin with a database exists at all
 *
 * Guaca runs on your machine and wants no account. It stays that way. What it
 * cannot do from there is hold an OAuth client: Google will not issue Gmail
 * access to an application whose client secret ships inside an open-source
 * download, and no amount of local cleverness changes that. A registered,
 * verified origin can hold that client. So this is a token broker and nothing
 * else: it authorizes services, keeps the refresh tokens encrypted, and hands
 * a short-lived access token to a machine the same person paired.
 *
 * It is opt-in from the app's side. An install that never pairs never talks to
 * it, and everything Guaca does without connectors it still does offline.
 */

/** `Env` is the Workers bindings interface from `env.d.ts`; this wraps it. */
type AppEnv = { Bindings: Env; Variables: { auth: Auth } };
type Ctx = Context<AppEnv>;

// Trailing slashes are the same route. The asset router in front of this
// drops them for files; a person who types `/app/` should not get a 404 from
// the half that does not.
const app = new Hono<AppEnv>({ strict: false });

// --- policy ---------------------------------------------------------------

/**
 * The same policy the marketing page ships in `_headers`, minus the parts that
 * only made sense for a page with no forms.
 *
 * `_headers` covers static assets. Worker responses get nothing unless it is
 * set here, and this is the half of the origin that holds tokens, so the
 * omission would have been the wrong way round. `form-action` is the one real
 * difference: these pages post, that one did not.
 *
 * It names every provider's consent screen as well as this origin, and that
 * is not decoration. A browser checks `form-action` against where a
 * submission ends up, following redirects, so authorizing with `'self'` alone
 * is a 302 to Google that the browser drops on the floor: the Worker log shows
 * a redirect nobody followed and the page shows a button that did nothing. The
 * list is exactly the consent screens in the catalog, with no wildcard and
 * nothing that is not the far end of a flow this origin starts.
 *
 * Loopback is on that list for the same reason and no other. Approving the
 * desktop app posts to `/consent` and this origin answers with a redirect to
 * the port Guaca is listening on, so the browser is again checking the far end
 * of a submission rather than its target. The port cannot be known in advance:
 * the app binds `127.0.0.1:0` and takes whatever the operating system gives
 * it, which is what makes the redirect safe in the first place, so the wildcard
 * is on the port and nowhere else. Nothing else on this origin posts anywhere
 * near it, and a page served from guaca.bot cannot read what a loopback server
 * answers.
 */
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
  // Every response below is either a person's grants or a redirect into an
  // OAuth flow. None of it is shared and none of it is worth a stale copy.
  if (!c.res.headers.has("Cache-Control")) {
    c.res.headers.set("Cache-Control", "private, no-store");
  }
});

/**
 * Origin checking on our own forms.
 *
 * Better Auth checks `trustedOrigins` on its own routes. The routes in this
 * file are ours, and every state-changing one of them is a form post, so a
 * page on another origin posting to them is the whole attack. `/api/auth/*` is
 * excluded because Better Auth's own check is stricter than this one, and
 * `/api/connectors` because the desktop app is not a browser and sends no
 * Origin header. That one is covered by `requireBearer` below instead.
 */
const formCsrf = csrf({
  origin: (origin, c) => origin === (c.env.SELFHOST_BEARER ? c.env.BETTER_AUTH_URL : new URL(c.req.url).origin),
});
app.use("/signin/*", formCsrf);
app.use("/signout", formCsrf);
app.use("/app/*", formCsrf);
app.use("/consent", formCsrf);

/**
 * The desktop app's API is bearer-only, and the bearer is an OAuth access
 * token rather than a session.
 *
 * It is the one part of this origin a browser has no business reaching, and
 * refusing a cookie here means the question of whether a cross-site form could
 * make a signed-in browser ask for a provider token does not need answering.
 * The paired app has no cookie jar and was always going to send this header.
 *
 * `linked` below is what turns the header into a person, and it is the reason
 * the access token is opaque rather than signed: it is validated against
 * storage on every call, so signing a machine out kills it now rather than
 * whenever it would have expired.
 */
app.use("/api/connectors/*", requireBearer);
app.use("/api/connectors", requireBearer);
app.use("/mcp", requireBearer);
app.use("/mcp/*", requireBearer);

app.use("*", async (c, next) => {
  c.set("auth", getAuth(c.env));
  await next();
});

// --- helpers --------------------------------------------------------------

/**
 * The posted form, or an empty one.
 *
 * `formData()` throws on a request with no body or a content type it does not
 * recognize, and a malformed post is not a server fault. Every handler below
 * already copes with a missing field by refusing the action, so handing it
 * nothing produces the same refusal rather than a 500.
 */
async function formOf(c: Ctx): Promise<FormData> {
  try {
    return await c.req.formData();
  } catch {
    return new FormData();
  }
}

/**
 * Refusing a bearer call in a way the caller can act on.
 *
 * A 401 with no `WWW-Authenticate` tells a client that it may not do this and
 * nothing about how it ever could. RFC 9728 section 5.1 puts the location of
 * the protected resource metadata in that header, and the MCP specification
 * makes it a MUST, so this is the first step of every sign-in a paired machine
 * or an MCP client performs. `error` distinguishes the two ways to fail: no
 * credential at all, versus one that storage has since stopped honoring.
 *
 * Naming the document is only half of it: the URL has to be one the document is
 * valid at. See `protectedResourceMetadataUrl` for why this is the path-inserted
 * form rather than the bare one it used to be.
 */
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

/** Who a paired machine is acting as, or nothing. */
interface Linked {
  userId: string;
  /**
   * The address the account signs in with, for the app and for nothing else.
   *
   * Guaca names the account a machine is paired to, because an operator can
   * hold two and no other value tells them apart. It reaches `/api/connectors`
   * and stops: `serveMcp` below does not pass it on, and `Caller` in
   * `worker/mcp.ts` has no field to receive it.
   */
  email: string;
  scopes: Set<string>;
}

/**
 * Turns the access token on the request into the person who authorized it.
 *
 * Through `/oauth2/userinfo` rather than by reading a row, because that
 * endpoint is the authorization server's own answer to "is this token still
 * good, and whose is it": it checks expiry, revocation and the client the token
 * was issued to, and it is the path a library upgrade keeps working. Doing it
 * here by hand would be this origin re-implementing the one thing it installed
 * a library for.
 *
 * `openid` is in the scope list purely to make this call legal. `email` is what
 * lets the app say which account a machine is linked to, and it goes no further
 * than the app: see `SCOPES` in `worker/auth.ts`. Anything the token was not
 * granted is absent rather than wrong.
 */
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
      // Put there by `customUserInfoClaims`, because OpenID does not carry it.
      scopes: new Set((info.scope ?? "").split(" ").filter(Boolean)),
    };
  } catch {
    // Expired, revoked, never ours, or not an access token at all. The app's
    // answer to every one of those is the same: sign in again.
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

/**
 * A redirect target taken from a query string, made safe.
 *
 * Anything that is not a path on this origin is dropped rather than corrected.
 * `//evil.example` is a protocol-relative URL that a browser follows off-site,
 * and it is the reason this cannot just be a `startsWith("/")`.
 */
function safeNext(value: string | undefined, fallback = "/app"): string {
  if (!value?.startsWith("/") || value.startsWith("//")) return fallback;
  return value;
}

/** Turn a Better Auth `{url}` answer into the redirect a form post wants. */
function redirectWith(headers: Headers, url: string | undefined, fallback: string): Response {
  const out = new Response(null, { status: 302, headers });
  out.headers.set("Location", url ?? fallback);
  return out;
}

// --- Better Auth ----------------------------------------------------------

/**
 * The client authentication method this origin accepts and does not advertise.
 *
 * `guaca-desktop` is a native public client: `migrations/0001_oauth_provider.sql`
 * seeds it with a `tokenEndpointAuthMethod` of `none` and `requirePKCE` set,
 * because an open-source download has nowhere to keep a secret. The token
 * endpoint accepts exactly that, and PKCE is what stands in for the secret.
 */
const PUBLIC_CLIENT_AUTH_METHOD = "none";

/**
 * The prompt the authorize endpoint refuses, and therefore does not advertise.
 *
 * `select_account` asks an authorization server to let the person choose which
 * of their accounts to continue with. There is nothing here to choose: a
 * session on this origin is one address, and the desktop app pairs one machine
 * to that one person. The plugin settles it the same way from the other end. It
 * refuses the value unless `selectAccount.page` names a screen to send the
 * person to, and this origin configures no such screen because it has no second
 * account to offer.
 */
const UNSUPPORTED_PROMPT = "select_account";

/**
 * The plugin's metadata, corrected to describe the one client this origin has.
 *
 * Every correction below exists for the same reason. The plugin decides what to
 * advertise from its registration settings, and this origin does not register
 * clients at runtime: it seeds its one client in a migration, on purpose, so
 * the consent screen names an application this origin chose rather than one a
 * stranger asserted. That client is public. It has no secret, and the plugin's
 * heuristics have no way to know it exists.
 *
 * Every endpoint the document names still comes from the plugin, so none of
 * them can drift from where the server actually is.
 */
async function withPublicClientCorrections(res: Response): Promise<Response> {
  if (res.status !== 200) return res;
  const body = (await res.json()) as Record<string, unknown>;

  /*
   * The methods the token endpoint really takes.
   *
   * The plugin advertises `none` only when `allowUnauthenticatedClientRegistration`
   * is on or an extension contributes a client discovery, and this origin does
   * neither, so the document offered three methods that all require a secret to
   * the only client there is, which has none.
   *
   * Dropping the field instead would not be neutral. RFC 8414 section 2 makes
   * `client_secret_basic` the default when it is absent, so a client that
   * follows the document sends an empty Basic header rather than the PKCE-only
   * request that works, and reads the refusal as its own misconfiguration. The
   * list is added to rather than replaced, so a confidential client registered
   * later is not locked out.
   */
  const methods = body.token_endpoint_auth_methods_supported;
  if (Array.isArray(methods) && !methods.includes(PUBLIC_CLIENT_AUTH_METHOD)) {
    body.token_endpoint_auth_methods_supported = [PUBLIC_CLIENT_AUTH_METHOD, ...methods];
  }

  /*
   * The methods the revocation endpoint really takes.
   *
   * The plugin fills this field and the introspection one from a single
   * `endpoint_auth_methods_supported` override, so configuration cannot move
   * one without moving the other, and the two endpoints do not agree.
   * `revokeEndpoint` refuses only when `client_id` is missing, while
   * `introspectEndpoint` refuses unless a secret arrives with it. The public
   * client can revoke a token here and genuinely cannot introspect one, so
   * this is the only one of the pair that is wrong, and the only one widened.
   *
   * Left as it is, this repeats the token endpoint's problem one field over,
   * and the consequence is worse than a failed call. Naming three
   * secret-bearing methods is a stronger statement than saying nothing, so a
   * client that follows the document concludes revocation is closed to it and
   * stops asking. What stays behind is a refresh token live on this server
   * until it expires, belonging to the machine that just signed out, because
   * the only client that would revoke it is the one the document locked out.
   */
  const revocationMethods = body.revocation_endpoint_auth_methods_supported;
  if (Array.isArray(revocationMethods) && !revocationMethods.includes(PUBLIC_CLIENT_AUTH_METHOD)) {
    body.revocation_endpoint_auth_methods_supported = [
      PUBLIC_CLIENT_AUTH_METHOD,
      ...revocationMethods,
    ];
  }

  /*
   * No ID token is ever issued here, so no algorithm is advertised for one.
   *
   * `disableJwtPlugin` removes the signing key, which leaves the plugin signing
   * ID tokens with the client secret as an HMAC key, and it advertises `HS256`
   * on that basis alone. What it never checks is whether a client has a secret
   * to key that HMAC with. `createIdToken` returns nothing at all when
   * `disableJwtPlugin` is set and `client.clientSecret` is null, and that is
   * every request this origin serves: the migration seeds the one client with
   * a NULL secret because Guaca ships as an open-source download.
   *
   * Left in, it is the one claim in the document that no client can act on. It
   * tells a client that asks for `openid` (a scope this origin does advertise,
   * because `openid` is what lets `/oauth2/userinfo` name a person) to expect a
   * signed ID token, which OpenID Connect Core section 3.1.3.3 makes required
   * once `openid` is granted. None arrives. The claim also contradicts the
   * field corrected just above: an HMAC keyed by a client secret cannot be
   * verified by a client whose only offered authentication method is `none`,
   * because such a client has no secret. Both cannot be true at once.
   *
   * Removing it says what this origin is, which is an OAuth 2.0 server with a
   * userinfo endpoint rather than an OpenID provider that issues tokens. What
   * would make the claim true again is seeding a client that has a secret, so
   * `worker.itest.ts` pins the removal to that column and fails if one appears.
   */
  delete body.id_token_signing_alg_values_supported;

  /*
   * The prompts the authorize endpoint really honors.
   *
   * `prompt_values_supported` is a fixed literal in the plugin. It names all
   * five OpenID Connect values with no option behind it and no regard for what
   * this server does with them, and four of the five are honored.
   * `select_account` is refused at the top of the authorize endpoint, before
   * the client or the redirect URI has even been looked at, and that ordering
   * is what makes advertising it worse than an ordinary broken promise.
   *
   * Every other refusal on that endpoint reaches the client. An invalid scope,
   * an unregistered redirect URI, `prompt=none` with nobody signed in: each is
   * a redirect to the loopback port carrying a registered error code, `state`
   * and `iss`, so the app waiting there learns what happened and can say so.
   * `select_account` instead redirects to this origin's own error page with
   * `unsupported_prompt_select_account`, which belongs to no error registry.
   * The browser stops on guaca.bot, the app holds its port open for a code that
   * is never coming, and the pairing hangs with nothing to report at either
   * end.
   *
   * So the value is removed rather than described differently. A client can
   * only choose a prompt the document offers, and the four left all lead
   * somewhere.
   */
  const prompts = body.prompt_values_supported;
  if (Array.isArray(prompts)) {
    body.prompt_values_supported = prompts.filter((prompt) => prompt !== UNSUPPORTED_PROMPT);
  }

  /*
   * No client here can end a session, so no endpoint is named for it.
   *
   * `end_session_endpoint` is a fixed literal in the plugin, like the prompt
   * list above: the URL is written into the document whatever the server is
   * configured to do, and `rpInitiatedLogoutEndpoint` then refuses every client
   * whose `enableEndSession` column is not set. That column is set at
   * registration, which is off in both directions here, and
   * `migrations/0001_oauth_provider.sql` does not name it when it seeds
   * `guaca-desktop`. So the one client this origin has, and the only one it can
   * ever have, is refused by name.
   *
   * The refusal is the same shape as `select_account` above, and lands the same
   * way. `enableEndSession` is read before `post_logout_redirect_uri` is looked
   * at, so there is no registered URI to fall back to and nothing is redirected
   * anywhere: a browser sent here stops on a 401 error page on guaca.bot
   * reading "The client is not allowed to initiate logout", and the app that
   * opened it is still waiting. Removing the URL is what stops a client from
   * starting a flow that has no way back.
   *
   * Nothing is lost by removing it. RP-Initiated Logout ends the browser
   * session at this origin, and the desktop app has no browser session here to
   * end: it holds a refresh token, which `revocation_endpoint` takes, and that
   * field is corrected above so it can. The hosted pages sign out through their
   * own `POST /signout`, which is a form on this origin and reads no metadata.
   * Seeding a client with `enableEndSession` is what would make the URL true
   * again, so `worker.itest.ts` pins the removal to the endpoint's refusal
   * rather than to this line.
   */
  delete body.end_session_endpoint;

  // The body is re-serialized, so the length the plugin set no longer describes it.
  const headers = new Headers(res.headers);
  headers.delete("content-length");
  return new Response(JSON.stringify(body), { status: res.status, headers });
}

// The plugin's own copies, mounted under the issuer. OpenID Connect Discovery
// derives `/api/auth/.well-known/openid-configuration` from this issuer and
// nothing else, so it is the first document many clients read. Both are named
// ahead of the catch-all below, which would otherwise answer them uncorrected.
app.get("/api/auth/.well-known/oauth-authorization-server", async (c) =>
  withPublicClientCorrections(await c.var.auth.handler(c.req.raw)),
);

app.get("/api/auth/.well-known/openid-configuration", async (c) =>
  withPublicClientCorrections(await c.var.auth.handler(c.req.raw)),
);

// Its own routes, including every endpoint of the authorization server the
// desktop app signs in to. Registered before anything else so nothing below
// can shadow a callback.
app.all("/api/auth/*", (c) => {
  if (c.env.SELFHOST_BEARER && c.req.path !== "/api/auth/callback/google") {
    return c.notFound();
  }
  return c.var.auth.handler(c.req.raw);
});

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

/**
 * Where a client finds out how to sign in here, at the root of the origin.
 *
 * RFC 8414 puts this document at `/.well-known/oauth-authorization-server` on
 * the issuer's origin, and the plugin mounts its copy under `basePath`, which
 * is `/api/auth`. A client that follows the specification looks at the root and
 * finds the marketing 404, so the same handler is exported here as well. It is
 * the plugin's own answer, not a second one written beside it: a hand-built
 * document is one deploy away from naming an endpoint that has moved.
 */
app.get("/.well-known/oauth-authorization-server", async (c) =>
  withPublicClientCorrections(await oauthProviderAuthServerMetadata(c.var.auth)(c.req.raw)),
);

/**
 * The same document at the place RFC 8414 actually derives for this issuer.
 *
 * The issuer is `https://guaca.bot/api/auth`, not the bare origin, and RFC 8414
 * section 3.1 builds the metadata URL by inserting the well-known segment
 * between the issuer's host and its path. So a client that does the derivation
 * literally asks for `/.well-known/oauth-authorization-server/api/auth`, which
 * is neither of the two paths above. Serving all three is the difference
 * between a client discovering this server and a client giving up, and they
 * are one handler, so none of them can drift from the others.
 */
app.get("/.well-known/oauth-authorization-server/api/auth", async (c) =>
  withPublicClientCorrections(await oauthProviderAuthServerMetadata(c.var.auth)(c.req.raw)),
);

/**
 * The same server, under the name an OpenID Connect client knows it by.
 *
 * This origin issues ID tokens and answers `/oauth2/userinfo`, and `openid` is
 * in the scopes above, so a client is entitled to look for OpenID Connect
 * Discovery. That document lives at the issuer, which is why the plugin's copy
 * at `/api/auth/.well-known/openid-configuration` already answers and is the
 * only path Discovery itself derives.
 *
 * RFC 8414 section 5 is the part that is easy to miss: a client that starts
 * from the origin rather than the issuer tries the root and the path-inserted
 * form under this name too, exactly as it does for the document above. The
 * edge logs have both, and both were the marketing 404. They are the plugin's
 * own answer rather than a hand-built one, for the same reason: a document
 * written beside the server is one deploy away from naming an endpoint that
 * has moved.
 */
const openIdConfiguration = async (c: Ctx) =>
  withPublicClientCorrections(await oauthProviderOpenIdConfigMetadata(c.var.auth)(c.req.raw));

app.get("/.well-known/openid-configuration", openIdConfiguration);
app.get("/.well-known/openid-configuration/api/auth", openIdConfiguration);

// --- this origin as a protected resource -----------------------------------

/**
 * Where an MCP client finds out which authorization server guards `/mcp`.
 *
 * RFC 9728, which the MCP authorization specification makes mandatory: a server
 * that refuses a call MUST say where the metadata lives, and that metadata MUST
 * name at least one authorization server. Without it a client meets a bare 401
 * and has nowhere to go, which is exactly what the edge logs showed: a client
 * tried `/mcp`, was refused, guessed at `/.well-known/mcp/server-card.json`,
 * and stopped. The refusal was correct and the connection was still impossible.
 *
 * `authorization_servers` is the issuer rather than the origin, because that is
 * what the document above calls itself. A client that takes this value and runs
 * RFC 8414 discovery on it lands on the alias registered right above.
 */
function protectedResourceMetadata(c: Ctx) {
  const origin = new URL(c.env.BETTER_AUTH_URL).origin;
  return c.json({
    // The canonical URI of the MCP server, which is what a client echoes back
    // as the RFC 8707 `resource` parameter when it asks for a token. Shared with
    // `protectedResourceMetadataUrl` below, because section 3.3 makes the two an
    // identity rather than a pair that happens to agree.
    resource: mcpResource(c.env),
    authorization_servers: [`${origin}/api/auth`],
    // Both scopes a token needs to get past `linked()` and `serveMcp`, not just
    // the one that is this origin's own; see `MCP_SCOPES`.
    scopes_supported: [...MCP_SCOPES],
    bearer_methods_supported: ["header"],
  });
}

/**
 * This origin's MCP server, named the way the metadata publishes it.
 *
 * No trailing slash: the specification asks for the shortest form that still
 * identifies the server. A token is accepted on the per-connection paths below
 * `/mcp` because `linked()` authenticates the token rather than matching its
 * audience against the request path.
 */
function mcpResource(env: Env): string {
  return `${new URL(env.BETTER_AUTH_URL).origin}/mcp`;
}

/**
 * Where the document above lives for the resource it describes.
 *
 * RFC 9728 section 3.1 builds this URL by inserting the well-known segment
 * between the resource identifier's host and its path, so the resource
 * `https://guaca.bot/mcp` is described at
 * `https://guaca.bot/.well-known/oauth-protected-resource/mcp`. Deriving it here
 * rather than writing it out is what keeps it correct: section 3.3 requires the
 * `resource` value in the document to be identical to the identifier this URL
 * was built from, and a client MUST discard the document when it is not.
 *
 * That requirement is why `unauthorized` no longer names the bare
 * `/.well-known/oauth-protected-resource`. The same document is served there,
 * but at that URL the identifier is the origin, not `${origin}/mcp`, so a client
 * that follows the specification had to throw away the only answer it was given
 * and had nowhere left to go: the exact failure the `WWW-Authenticate` header
 * exists to prevent.
 */
function protectedResourceMetadataUrl(env: Env): string {
  const resource = new URL(mcpResource(env));
  return `${resource.origin}/.well-known/oauth-protected-resource${resource.pathname}`;
}

/**
 * The document at the URL the resource identifier derives, which is the one the
 * `WWW-Authenticate` header names and the only one it is valid at.
 *
 * RFC 9728 section 3.1 derives this the same way RFC 8414 does a metadata URL
 * from an issuer, so a client holding `https://guaca.bot/mcp` and a client
 * following the header both arrive here, and section 3.3's identity check passes
 * for both. See `protectedResourceMetadataUrl`.
 */
app.get("/.well-known/oauth-protected-resource/mcp", protectedResourceMetadata);

/**
 * The same document at the root, for clients that look there anyway.
 *
 * This is deliberately not what the header advertises. At this URL the resource
 * identifier is the bare origin, so the `resource` value inside does not match
 * what section 3.3 derives and a client that runs that check MUST discard it. It
 * stays because a client that guesses the root instead of reading the header
 * still finds the authorization server from it, and a 404 would give that client
 * nothing at all. Nothing this origin says points here.
 */
app.get("/.well-known/oauth-protected-resource", protectedResourceMetadata);

// --- signing in -----------------------------------------------------------

/**
 * Signing in is an address and a code, and nothing else.
 *
 * Two steps, two pages, one cookie between them. The address is not in the
 * URL of the second page: it goes in a signed cookie scoped to `/signin` and
 * expiring with the code, so a refresh, a back button and a shared link all
 * behave, and the address does not end up in a log or a browser history.
 *
 * There is no provider sign-in. Identity here is the email a code reached,
 * and every provider is a grant hanging off it. See `connectors.ts`.
 */
const PENDING = "guaca_signin_email";

const errors: Record<string, string> = {
  required: "Sign in to continue.",
  send: "That code could not be sent. Check the address and try again.",
  code: "That code is not right, or it has expired. Ask for another.",
  expired: "That code has expired. Ask for another.",
};

/** Where the pending address is kept, and for how long. */
function pendingCookie(env: Env) {
  return {
    path: "/signin",
    httpOnly: true,
    // Off on localhost, where the origin is not HTTPS and a secure cookie is
    // silently dropped, which looks exactly like a broken sign-in.
    secure: env.BETTER_AUTH_URL.startsWith("https://"),
    sameSite: "Lax" as const,
    maxAge: CODE_TTL_MINUTES * 60,
  };
}

/**
 * Where an authorization request goes when nobody is signed in yet.
 *
 * `oauthProvider` is configured with this page as its `loginPage`, and it sends
 * the browser here with the whole authorization request re-signed into the
 * query string rather than with a `next`. Dropping that would strand the
 * operator on the connectors page with Guaca still waiting on a port, so it is
 * turned back into the `next` this page already knows how to carry: the
 * authorize endpoint, with its parameters and their signature intact.
 *
 * `sig` is the tell, and it is the plugin's own name for the signature over
 * everything else in the query. Nothing else on this origin puts one there.
 */
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
    // Round-tripped through the form rather than the button's href, so it
    // survives the post that follows. Losing it here is invisible: sign-in
    // works, and the person lands on the wrong page still holding a code.
    next,
    error: errors[c.req.query("error") ?? ""],
  });
  return page(c, body, "Sign in to Guaca");
});

/**
 * Ask for a code.
 *
 * Registered before any other `/signin/*` post so the path is unambiguous.
 * Sending is awaited rather than deferred: the person is looking at the form,
 * and a provider that refused the address is worth saying out loud now. That
 * makes the post slow enough, around two seconds against a real mail
 * provider, that a second one is a normal thing for a person to do, and this
 * page has no JavaScript to gray the button out.
 *
 * So a duplicate post is folded into the one that is already on its way
 * rather than answered with a second code that would silently kill the first.
 * See `codes.ts`. The address still gets its cookie and the same redirect,
 * because from where the person is standing it was the same request.
 */
app.post("/signin/email", async (c) => {
  if (c.env.SELFHOST_BEARER) return c.notFound();
  const form = await formOf(c);
  const email = String(form.get("email") ?? "")
    .trim()
    .toLowerCase();
  const next = safeNext(form.get("next")?.toString());

  if (await codeAlreadySent(c.env, email)) {
    // Worth seeing: a deployment where this is most posts is one where the
    // send is slow enough that everybody presses the button twice.
    console.log("sign-in code asked for twice; the first one still stands");
  } else {
    try {
      await c.var.auth.api.sendVerificationOTP({
        body: { email, type: "sign-in" },
        headers: c.req.raw.headers,
      });
    } catch (error) {
      // A malformed address. It does not say whether an account exists, and
      // should not: this form is public.
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
  // No pending address means a code was never asked for here, or it has been
  // long enough that the cookie went with it. Either way the answer is the
  // form that starts the flow, not an empty box.
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
    // Wrong, expired, or the third wrong guess, which spends the code. The
    // page says all three the same way, because the difference is only useful
    // to someone working through codes that are not theirs.
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

// --- connectors -----------------------------------------------------------

app.get("/app", async (c) => {
  const session = await c.var.auth.api.getSession({ headers: c.req.raw.headers });
  if (!session) return c.redirect("/signin?error=required&next=%2Fapp", 302);

  const providers = configuredProviders(c.env as unknown as Record<string, unknown>);
  const connections = await readConnections(c.env, c.var.auth, session.user.id, providers);

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

  /**
   * What the provider or the linking actually refused, when it says.
   *
   * Better Auth appends its own `error` to the failure redirect, and this page
   * used to drop it and print "That did not complete. Nothing was changed." for
   * every one of them. That sentence is true and useless: it names no cause and
   * suggests no action, and the one failure people actually hit has a specific
   * cause and a specific fix.
   *
   * An unrecognised code is shown rather than swallowed. A person reporting
   * "it says invalid_scope" can be helped; a person reporting "it says nothing
   * happened" cannot.
   */
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

  /**
   * Which identity this is about, or none for a new one.
   *
   * The card that authorizes a fresh account sends no connection, and that is
   * the difference between adding a second Google and editing the first. Read
   * from this person's own connections rather than trusted, so a posted id that
   * is not theirs is simply not found.
   */
  const connection = form.get("connection")
    ? await readConnection(
        c.env,
        c.var.auth,
        session.user.id,
        [provider],
        String(form.get("connection")),
      )
    : undefined;
  if (form.get("connection") && !connection) return c.redirect("/app?state=failed", 302);

  // Partial and older grants still need revocation when their capability is removed.
  const held = parseScopes(connection?.scope);
  const granted = new Set(
    provider.capabilities
      .filter((capability) => capability.scopes.some((scope) => held.has(scope)))
      .map((capability) => capability.id),
  );

  if (desired.size === 0) {
    // Nothing ticked on a card that holds nothing is not an instruction.
    // Answering it as a disconnect was a page that reloaded and claimed to
    // have revoked a grant that never existed, which reads as a broken
    // button. Say what is missing instead.
    if (!connection) return c.redirect("/app?state=nothing", 302);

    // With a grant standing, an empty tick list is a disconnect said a
    // different way. Routing it through the OAuth flow would send someone to a
    // consent screen to agree to nothing and come back with the old grant
    // intact.
    return disconnect(c, provider.id, connection?.id);
  }

  /**
   * Giving a capability back means revoking first.
   *
   * Neither provider narrows a grant in place: Google's incremental
   * authorization only ever adds, and GitHub's scopes accumulate on the app.
   * Asking for a smaller set and calling it done would leave the larger grant
   * live while this page claimed otherwise, which is the one lie this page
   * must not tell. So the grant is handed back, and the authorization that
   * follows rebuilds exactly what was ticked.
   *
   * Scoped to one connection: revoking the work Google because the personal one
   * was narrowed would be the same lie in the other direction.
   */
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
            // Google issues a refresh token on the first consent only, unless
            // asked again. Without one the grant dies the first time the
            // access token expires, and the paired machine starts failing for
            // no visible reason. GitHub's tokens do not expire and it has no
            // equivalent, so it is not sent parameters it will ignore.
            //
            // `select_account` is the other half and was missing. `consent`
            // alone forces the consent screen and still silently uses whichever
            // Google the browser is already signed into, so somebody with a
            // work account and a personal one could authorize the same one
            // repeatedly and never be offered the other. Two identities are two
            // grants here — see `connections.ts` — and this is what lets a
            // person reach the second.
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

  // Named, and checked against this person's own. Disconnecting "Google" when
  // two are authorized would be a button whose effect depends on row order.
  const connection = await readConnection(
    c.env,
    c.var.auth,
    session.user.id,
    [provider],
    String(form.get("connection") ?? ""),
  );
  if (!connection) return c.redirect("/app?state=failed", 302);

  return disconnect(c, provider.id, connection.id);
});

// --- linking the desktop app ----------------------------------------------

/**
 * The consent screen, which is the only page in this flow this origin writes.
 *
 * Everything before it is the `oauthProvider` plugin: `/api/auth/oauth2/authorize`
 * checks the client, the redirect URI and the PKCE challenge, and then sends the
 * browser here with the request re-signed into the query string. That signature
 * is why this page can round-trip the parameters through a form without being
 * able to change them: `oauth2Consent` verifies it before it mints anything.
 *
 * There is a real screen here rather than `skipConsent` on the client, because
 * this is the moment the operator confirms a machine is theirs. Approving it is
 * the act that lets a copy of Guaca mint Google access tokens on their account,
 * and a flow that silently redirected back would be a link anybody could send.
 */
app.get("/consent", async (c) => {
  const query = new URL(c.req.url).search.slice(1);
  if (!query) return c.redirect("/app", 302);

  const session = await c.var.auth.api.getSession({ headers: c.req.raw.headers });
  if (!session) {
    // Back to exactly this URL, signature and all. The plugin already sent the
    // signed request here, so losing it would mean starting the flow again.
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
      // Every scope that was asked for, or none. There is nothing to tick on
      // the page: the app asks for one set and the answer is yes or no.
      body: { accept, oauth_query: query },
      headers: c.req.raw.headers,
      /**
       * A `Request`, because accepting re-runs the authorization request and
       * that half refuses to run without one. Rebuilt rather than forwarded:
       * `formOf` above has already read this request's body, and handing over a
       * spent one is a failure that would only appear the first time the
       * library decided to look at it.
       */
      request: new Request(c.req.url, { method: "POST", headers: c.req.raw.headers }),
    });

    /**
     * Where the browser goes next, however the answer arrives.
     *
     * Handing the call a `Request` makes Better Auth answer with a `Response`,
     * and without one it answers with the object. Both carry the same thing.
     * `url` is the field the implementation returns and `redirect_uri` is the
     * one its OpenAPI block documents, so both are read rather than betting on
     * which of the two the library eventually settles on.
     */
    const payload = (answered instanceof Response ? await answered.json() : answered) as {
      url?: string;
      redirect_uri?: string;
    } | null;
    redirect = payload?.url ?? payload?.redirect_uri;
  } catch {
    // An expired or tampered request. The plugin refuses to build a redirect
    // for one, and there is nowhere safe to send the browser, so it stays here.
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

  // The far end is a loopback port on the operator's own machine, which is why
  // `form-action` names it. A denial redirects too, carrying `access_denied`,
  // so Guaca says it was refused rather than sitting there until it times out.
  if (!redirect) return c.redirect("/app?state=failed", 302);
  return c.redirect(redirect, 302);
});

/**
 * What each scope is, in the operator's words.
 *
 * The consent screen is the one place a person decides whether to hand a
 * machine an account, so the list on it has to say what the machine gets, not
 * repeat the identifiers. A scope with no line here is shown as itself rather
 * than hidden: a screen that quietly omits what it does not recognize is a
 * screen that can be made to under-report.
 */
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

// --- the desktop app's API ------------------------------------------------

/**
 * This origin as an MCP server.
 *
 * The other two endpoints below describe what an account holds and hand over a
 * provider token. This one does the work instead, and it exists because of a
 * boundary the app draws rather than one this origin needs: a plugin's tools
 * are called by Guaca on behalf of an agent, and the grant never reaches a
 * prompt, a transcript or a sandbox. Serving tools keeps the Google token here,
 * beside the refresh token that produced it, instead of putting a live
 * credential on a laptop for an hour at a time.
 *
 * Authenticated exactly like the endpoints below: the account's own access
 * token, carrying `connectors`. See `worker/mcp.ts`.
 */
app.post("/mcp", async (c) => serveMcp(c, undefined));

/**
 * The same server, scoped to one authorized identity.
 *
 * A crew names the connection it means in the path, because a person can hold
 * two Google accounts and a group has to be able to say which one. Bare `/mcp`
 * above is every connection at once and stays: it is what plugins connected
 * before this existed still dial, and it is the right answer for an account
 * with one identity, which is most of them.
 */
app.post("/mcp/:connection", async (c) => serveMcp(c, c.req.param("connection")));

async function serveMcp(c: Ctx, connection: string | undefined) {
  const who = await linked(c);
  if (!who) return unauthorized(c, "invalid_token");
  if (!who.scopes.has(CONNECTORS_SCOPE)) return c.json({ error: "insufficient_scope" }, 403);
  // `who.email` is deliberately not passed. Everything this call produces is
  // read by a model: a tool list, a tool result, a refusal. The account's own
  // address has no bearing on any of it, and the address an agent is told is
  // the connection label, which is the Google or GitHub identity the tool acts
  // as. `Caller` has no field for the other one, so this is a boundary rather
  // than a habit.
  return mcp.handle(c.req.raw, c.env, c.var.auth, {
    userId: who.userId,
    connection,
  });
}

/**
 * What this machine may reach, and a token to reach it with.
 *
 * Authenticated by the OAuth access token the sign-in handed back, sent as a
 * bearer. Deliberately small: the app needs to know which capabilities exist
 * and to get a live access token, and nothing here should tempt it to keep
 * one.
 */
app.get("/api/connectors", async (c) => {
  const who = await linked(c);
  if (!who) return unauthorized(c, "invalid_token");
  if (!who.scopes.has(CONNECTORS_SCOPE)) return c.json({ error: "insufficient_scope" }, 403);

  const providers = configuredProviders(c.env as unknown as Record<string, unknown>);
  const connections = await readConnections(c.env, c.var.auth, who.userId, providers);

  /**
   * `connections` is the field that matters, and `providers` is kept beside it.
   *
   * A person can authorize the same provider twice — a work Google and a
   * personal one — and those are two grants with two ids, which is what a group
   * has to choose between. `providers` still says what each one *could* hold,
   * because the app draws the catalogue before anything is authorized and a
   * list of nothing is not an answer to "what is this for".
   *
   * `user` is this origin's own account address, and it is here for one line in
   * Settings: which Guaca account this machine is paired to. That is a different
   * question to which mailbox a tool acts on, which is `connection.label`, and
   * an operator with two accounts cannot answer the first from anything else.
   * It is the one place the address crosses this boundary. The MCP server does
   * not carry it, so no agent, tool list or transcript ever sees it.
   */
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
        // True when any connection for this provider has it, which is what the
        // Settings pane is reporting: whether the account can do this at all.
        granted: connections.some(
          (connection) =>
            connection.providerId === provider.id && connection.capabilities.has(capability.id),
        ),
      })),
    })),
  });
});

/**
 * A live access token for one provider.
 *
 * POST because it is not a read: an expired token is refreshed at the provider
 * before it is returned. The refresh token itself never leaves this origin,
 * which is the only reason storing it here is better than storing it on the
 * machine that will use it.
 */
app.post("/api/connectors/:provider/token", async (c) => {
  const who = await linked(c);
  if (!who) return unauthorized(c, "invalid_token");
  if (!who.scopes.has(CONNECTORS_SCOPE)) return c.json({ error: "insufficient_scope" }, 403);

  const provider = findProvider(c.req.param("provider"));
  if (!provider) return c.json({ error: "unknown_provider" }, 404);

  // A connection may be named in the query, for an account holding two of the
  // same provider. Unnamed takes the first, which is what this endpoint did
  // before connections existed and is right for the one-identity case.
  const all = await readConnections(c.env, c.var.auth, who.userId, [provider]);
  const asked = c.req.query("connection");
  const grant = asked
    ? all.find((connection) => connection.id === asked)
    : all.find((connection) => connection.capabilities.size > 0);
  if (!grant || grant.capabilities.size === 0) return c.json({ error: "not_authorized" }, 409);

  try {
    // No headers, deliberately. `getAccessToken` resolves the user from the
    // session when a request carries one and refuses outright when a request
    // carries headers with no session in it, which is every call from a paired
    // machine. Naming the user is the server-side path, and `accountId` is one
    // this person's own grants produced a line above, so it cannot address
    // somebody else's row.
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
    // The grant is gone at the provider, or the refresh token was rejected.
    // Either way the person has to authorize again, and saying so is more use
    // than a 500.
    return c.json({ error: "grant_expired" }, 409);
  }
});

// --- everything else ------------------------------------------------------

app.notFound(async (c) => {
  // Assets are matched before this Worker runs, so reaching here means no file
  // exists either. Hand it back so the marketing build's 404 page answers.
  const asset = await c.env.ASSETS.fetch(c.req.raw);
  if (asset.status !== 404) return asset;
  return c.text("Not found", 404);
});

app.onError((error, c) => {
  // A rejected origin check is a decision, not a failure. Hono raises it as an
  // exception, and swallowing it into a 500 would report a working defense as
  // a broken server, which is exactly the wrong way round for the one signal
  // worth watching here.
  if (error instanceof HTTPException) return error.getResponse();
  console.error("worker error", { path: new URL(c.req.url).pathname, error });
  return c.text("Something went wrong", 500);
});

// --- shared route bodies --------------------------------------------------

async function disconnect(c: Ctx, providerId: string, accountRowId: string | undefined) {
  if (!accountRowId) return c.redirect("/app?state=disconnected", 302);
  const revoked = await revokeIfPossible(c, providerId, accountRowId);
  if (!revoked) return c.redirect("/app?state=revoke-failed", 302);
  await forgetGrant(c.env, accountRowId);
  return c.redirect("/app?state=disconnected", 302);
}

/**
 * Hand the token back, and say whether it worked.
 *
 * A grant with no token left is already as revoked as this origin can make it,
 * so that counts as success. Anything else failing has to stop the disconnect:
 * clearing our row while the provider still lists the grant would leave a
 * person believing they had taken access away.
 */
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
