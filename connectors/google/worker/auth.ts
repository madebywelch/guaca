import { oauthProvider } from "@better-auth/oauth-provider";
import { betterAuth } from "better-auth";
import { drizzleAdapter } from "better-auth/adapters/drizzle";
import { emailOTP } from "better-auth/plugins";
import { drizzle } from "drizzle-orm/d1";

import { PROVIDERS } from "./connectors";
import { CODE_TTL_MINUTES, sendSignInCode } from "./email";
import { schema } from "./schema";

/**
 * The Better Auth instance, built per Worker isolate.
 *
 * It cannot be a module constant: the D1 binding arrives on `env`, which only
 * exists once a request does. Building one per request would instead re-run
 * plugin setup on every page load, so it is cached against the `env` object,
 * which workerd keeps for the life of the isolate. A `WeakMap` rather than a
 * module-level variable so a test can hand in a second environment without
 * getting the first one's database.
 */
const instances = new WeakMap<object, Auth>();

export type Auth = ReturnType<typeof build>;

export function getAuth(env: Env): Auth {
  const cached = instances.get(env);
  if (cached) return cached;
  const auth = build(env);
  instances.set(env, auth);
  return auth;
}

/**
 * How long a sign-in lasts, in days.
 *
 * Better Auth's default is the same seven days, so this changes nothing today.
 * It is set explicitly because `/privacy` states the number: a default is not a
 * promise, and a library upgrade that moved it would quietly turn a sentence in
 * the privacy policy into a false one, which is the one kind of drift that page
 * cannot absorb.
 */
export const SESSION_TTL_DAYS = 7;

/**
 * How long a paired machine may be away and still come back on its own.
 *
 * Thirty days is the industry number and the plugin's own default, and it is
 * named here because two settings have to agree on it: the lifetime of a
 * refresh token, and how long a rotated one still answers with what it minted.
 * See `refreshTokenReuseInterval` below for why those are the same number.
 *
 * It is not bounded by `SESSION_TTL_DAYS`, though the pairing is approved from a
 * browser session, only because `patches/@better-auth__oauth-provider@1.7.1.patch`
 * stops a refresh binding new tokens to that session once it has ended. Without
 * it every machine was signed out a week after pairing while its refresh still
 * answered 200. Re-check the patch on every upgrade of the plugin.
 */
export const REFRESH_TOKEN_TTL_SECONDS = 30 * 24 * 60 * 60;

/**
 * The options that decide what tables exist.
 *
 * Split out so `schema.test.ts` can ask Better Auth what shape it expects
 * without a database to build a real instance against. Plugins are the reason
 * this matters: `oauthProvider` brings six tables with it, and a plugin added
 * here without a migration is a 500 on the first sign-in and nowhere earlier.
 */
export const ACCOUNT_OPTIONS = {
  /**
   * These rows hold live provider refresh tokens, which are the whole asset.
   * Encrypting them means a D1 read, a backup, or a leaked snapshot is not
   * the same event as a compromised Gmail account.
   */
  encryptOAuthTokens: true,
  accountLinking: {
    enabled: true,
    // Both verify the email they assert, which is what this list is asking.
    trustedProviders: ["google", "github"],
    /**
     * The connected account is not a claim about who you are.
     *
     * Better Auth refuses by default to link a provider account whose email
     * differs from the session's, because normally a linked account is a way
     * in: link one you do not own, sign in with it, take the account. Here it
     * is not. Nobody can sign in with a provider on this origin at all, so a
     * linked Google account is only ever a grant hanging off an address that
     * already received a code.
     *
     * Without this, signing in as you@work.com and connecting a Gmail account
     * fails with `LINKING_DIFFERENT_EMAILS_NOT_ALLOWED` after the consent
     * screen, which is both the common case and the worst possible moment.
     */
    allowDifferentEmails: true,
  },
};

/**
 * The `emailOTP` endpoints, closed to the outside.
 *
 * Every one of them is reachable from this Worker through `auth.api`, which
 * is how `/signin/email` and `/signin/code` use them. None of them needs to be
 * an HTTP route, and leaving them mounted would hand out a public way to make
 * this origin email an arbitrary address, and a second door to a code that
 * skips the checks on the form: see `codes.ts`. A path here is 404 to a
 * browser and unchanged to a server call.
 *
 * A denylist because Better Auth has no allowlist, so `auth.test.ts` walks the
 * plugin's endpoints and fails if an upgrade mounts one this does not name.
 */
export const DISABLED_PATHS = [
  "/email-otp/send-verification-otp",
  "/email-otp/check-verification-otp",
  "/email-otp/verify-email",
  "/sign-in/email-otp",
  "/email-otp/request-password-reset",
  "/forget-password/email-otp",
  "/email-otp/reset-password",
  "/email-otp/request-email-change",
  "/email-otp/change-email",
];

/**
 * The desktop app, as an OAuth client.
 *
 * Fixed and public: there is one application on the other end of this flow and
 * it ships as an open-source download, so it has no secret and needs none. PKCE
 * is what binds an authorization code to the copy of Guaca that asked for it,
 * and `token_endpoint_auth_method: "none"` is the honest name for a client that
 * cannot keep one. The row is seeded by the migration rather than registered at
 * runtime, so the consent screen names an application this origin chose instead
 * of one a stranger asserted.
 */
export const DESKTOP_CLIENT_ID = "guaca-desktop";

/**
 * What Guaca may ask for, and nothing wider.
 *
 * `openid` is not decoration: it is what lets `/oauth2/userinfo` turn a bearer
 * token back into a person, which is how `/api/connectors` authenticates
 * without this origin hand-rolling token validation. `offline_access` is the
 * refresh token, without which a paired machine asks to be signed in again
 * every hour.
 *
 * `email` is the address a sign-in code reached, and it is granted for exactly
 * one job: naming which Guaca account a machine is paired to. An operator can
 * hold two, and the failure that costs the most time is a laptop linked to the
 * account that does not hold the grant they are looking at in a browser.
 * Nothing else on either side can tell those apart.
 *
 * It stops at the desktop app. `/api/connectors` carries it because Settings
 * draws it; the MCP server does not, and `Caller` in `worker/mcp.ts` has no
 * field for it to arrive in. What an agent is told about an identity is the
 * connection label, which is the Google or GitHub address the grant belongs to
 * and says which mailbox a tool will act on. The account address has no bearing
 * on any call a model makes, so it is not in a tool list, a tool result or a
 * prompt. `worker.itest.ts` checks that the MCP surface does not carry it.
 *
 * The list must equal the seeded client's `scopes` column, because
 * `oauthProvider` validates against `client.scopes ?? opts.scopes` and the row
 * wins. `worker.itest.ts` compares the two.
 */
export const SCOPES = ["openid", "email", "offline_access", "connectors"] as const;

/** The one scope that is this origin's own, rather than OpenID's. */
export const CONNECTORS_SCOPE = "connectors";

/**
 * The scopes a token needs to reach `/mcp`, and therefore the ones the
 * protected resource metadata has to ask a client for.
 *
 * `connectors` is the authorization, checked by `serveMcp`. `openid` is not
 * decoration next to it: `linked()` resolves the bearer token to a person
 * through `/oauth2/userinfo`, and that endpoint refuses a token whose scope
 * omits `openid`, which `linked()` can only report as a bare `401
 * invalid_token`.
 *
 * The document used to name `connectors` alone, so a client that read it and
 * asked for exactly what it named was issued a token, on the advertised scope,
 * that the resource then refused on every call. It failed after consent rather
 * than before it, which is the worst place for it: the person had already said
 * yes, and nothing in the refusal names the scope that was missing. The desktop
 * app never met it because it asks for all four scopes regardless.
 */
export const MCP_SCOPES = ["openid", CONNECTORS_SCOPE] as const;

/**
 * The grants this server implements, and therefore the ones it advertises.
 *
 * The plugin's default is `authorization_code`, `client_credentials` and
 * `refresh_token`, and it published that list on all six metadata documents
 * without consulting anything about this deployment. Two of the three are real.
 * `client_credentials` never was: `handleClientCredentialsGrant` refuses any
 * client whose `tokenEndpointAuthMethod` is `none`, and that is the seeded
 * client, and it is the only client there will ever be. Dynamic registration is
 * off in both directions, so no confidential client can appear later and make
 * the claim true.
 *
 * Nor would one have anywhere to go. `client_credentials` is a token with no
 * person behind it, and everything this origin guards belongs to a person:
 * `/api/connectors` turns a bearer token into a user through
 * `/oauth2/userinfo`, and the plugin bars `openid`, `email` and
 * `offline_access` from this grant as user-delegated. The scope that is left is
 * `connectors`, which resolves a person or refuses.
 *
 * Setting this rather than rewriting the JSON is what makes the document and
 * the endpoint one decision: `tokenEndpoint` calls `getSupportedGrantTypes`
 * before it dispatches, so a grant removed here stops being served and stops
 * being advertised in the same edit, and the refusal changes from
 * `unauthorized_client` to `unsupported_grant_type`, which is what RFC 6749
 * section 5.2 means for a grant the server does not implement.
 *
 * The list must equal the seeded client's `grantTypes` column, for the same
 * reason `SCOPES` must equal its `scopes` column. `worker.itest.ts` compares
 * the two.
 */
export const GRANT_TYPES = ["authorization_code", "refresh_token"] as const;

/**
 * The claims this server can actually put in front of a client.
 *
 * `claims_supported` is the plugin's own construction, not a default it copied:
 * `oauthProvider` builds the set as a fixed eight (`sub`, `iss`, `aud`, `exp`,
 * `iat`, `sid`, `scope`, `azp`) plus whichever `STANDARD_CLAIMS` the granted
 * scopes reach, and it does that without consulting `disableJwtPlugin`. Six of
 * that fixed eight describe an ID token's payload, and this origin issues no ID
 * token: #23 removed `id_token_signing_alg_values_supported` from all six
 * documents for exactly that reason, and left the claim list that same absent
 * token was going to carry.
 *
 * What is left once the vehicle is gone is `/oauth2/userinfo`, and it can
 * supply four things. `userNormalClaims` returns `sub` always and resolves a
 * `STANDARD_CLAIMS` entry only when the granted scopes include the scope that
 * owns it, which here is `email` reaching `email` and `email_verified`. The
 * fourth is `scope`, and it is this origin's own: `customUserInfoClaims` below
 * writes it so `/api/connectors` can check what a token was issued for. The
 * other six have no resolver anywhere in the plugin and no second vehicle to
 * arrive in, because `disableJwtPlugin` leaves access tokens opaque as well.
 *
 * The field is not only advertisement, which is why this is the option rather
 * than another line in `withPublicClientCorrections`. `getSupportedClaims`
 * feeds `getRequestedUserInfoClaims`, which bounds the `claims` request
 * parameter at both `/oauth2/authorize` and `/oauth2/consent`. Left wide, a
 * client reads `sid` from the document, asks for it by name, is told nothing is
 * wrong (the filter drops unknown names silently rather than refusing), watches
 * the consent screen accept it, and receives a response without it. Narrowing
 * here closes the document and that allowlist in one edit, so the two cannot
 * disagree later. Nothing that worked stops working: the six removed names had
 * no resolver to reach in the first place.
 *
 * Adding a claim source means adding its name here. `worker.itest.ts` pins the
 * list to what `/oauth2/userinfo` really answers with, so a contributor that
 * starts emitting a claim without advertising it fails there.
 */
export const USERINFO_CLAIMS = ["sub", "email", "email_verified", "scope"] as const;

export function authPlugins(env?: Env) {
  return [
    /**
     * This origin, as an authorization server.
     *
     * The desktop app signs in the way RFC 8252 says a native app should:
     * authorization code with PKCE, redirected to a loopback port the app bound
     * before it asked. That is a deliberate replacement for the device grant
     * this origin used to offer. A device code is a bearer secret carried by a
     * human, and RFC 8628 section 5.4 says what follows: somebody talks an
     * operator into approving a code they did not generate and walks away with
     * a token that mints Gmail access tokens. A loopback redirect cannot be
     * phished across machines, because the code is delivered to 127.0.0.1 on
     * the machine that started the flow and is useless without the verifier
     * that never left it.
     *
     * The plugin implements RFC 8252 section 7.3 port variance, which is the
     * part that makes this work at all: the app binds `127.0.0.1:0`, takes
     * whatever port the operating system hands back, and the registered
     * redirect URI matches on scheme and host while ignoring the port.
     */
    oauthProvider({
      scopes: [...SCOPES],
      // The two grants this server implements. Left unset, the plugin
      // advertises its default third, `client_credentials`, which no client
      // here can use; see `GRANT_TYPES` above.
      grantTypes: [...GRANT_TYPES],
      // The claims `/oauth2/userinfo` can answer with, and therefore the only
      // ones worth naming. See `USERINFO_CLAIMS` above: the plugin's own list
      // describes an ID token this origin never issues, and the same field
      // bounds the `claims` request parameter, so this narrows the document and
      // that allowlist together.
      advertisedMetadata: { claims_supported: [...USERINFO_CLAIMS] },
      // Where the authorize endpoint sends someone who is not signed in, and
      // where it sends them to say yes. Both are this origin's own pages.
      loginPage: "/signin",
      consentPage: "/consent",
      // One client, seeded in the migration. Leaving registration open would
      // let anybody create client rows on a database whose job is holding
      // other people's refresh tokens.
      allowDynamicClientRegistration: false,
      allowUnauthenticatedClientRegistration: false,
      // Cached for the life of the isolate, and immutable through the CRUD
      // endpoints, which is what "first-party" should mean here.
      cachedTrustedClients: new Set([DESKTOP_CLIENT_ID]),
      // An hour on the access token, thirty days on the refresh token. The
      // access token is what a stolen file yields, so it is the short one.
      accessTokenExpiresIn: 60 * 60,
      refreshTokenExpiresIn: REFRESH_TOKEN_TTL_SECONDS,
      /**
       * How long a rotated refresh token still answers with what it minted.
       *
       * Refresh tokens rotate here, and the plugin treats a token presented
       * twice as theft: `handleRefreshTokenGrant` calls `invalidateRefreshFamily`,
       * which deletes every refresh and access token this user holds for this
       * client. That is the correct response to a stolen token and the wrong
       * response to a dropped packet, and nothing in the request distinguishes
       * them. The interval is what tells them apart.
       *
       * Left at the plugin's default of `0`, the replay cache is off entirely:
       * `storeRefreshTokenRotationReplay` returns before writing, so
       * `rotationReplayExpiresAt` is never set and
       * `isWithinRefreshTokenReuseInterval` cannot return true for any row.
       * Every second presentation therefore takes the theft branch. A machine
       * whose token response was lost in flight retries with the only token it
       * has, and that retry is what destroys the grant: not the network, the
       * recovery from it. The refresh the client stored is deleted too, so
       * there is nothing left to retry with and no error a client can act on.
       * The operator sees a machine that was paired yesterday asking to sign in
       * again, with a 400 an hour after the last success and no cause visible
       * from either end.
       *
       * The interval is the refresh token's own lifetime because no shorter
       * bound is about anything. What it measures is how long a machine takes to
       * notice it never got an answer, and a laptop that suspends mid-request
       * notices when it wakes. The two outages this setting exists for had
       * gaps of two hours forty-six minutes (2026-09-15, last success 07:44,
       * first refusal 10:30) and twenty-one minutes (2026-09-19, 06:23 and
       * 06:44). Thirty seconds, which is what this was, covers a lost packet
       * and nothing a laptop does; every number between the two is a guess
       * about sleep.
       *
       * The long window costs little because the stored response invalidates
       * itself. A replay is answered with a frozen snapshot of one rotation,
       * and the refresh token inside it is revoked the moment the real client
       * rotates past it, while the access token inside it dies an hour after
       * it was minted. A replay that arrives once the chain has moved on
       * therefore hands back credentials that are already spent, and one that
       * arrives before then hands back what whoever holds the predecessor
       * could have had by presenting it normally. What is given up is the
       * theft signal in the late case; what is bought is that a machine which
       * slept at the wrong moment is still paired when it wakes. Two false
       * positives in five days against no true one is the trade.
       *
       * `refreshToken.expiresAt` is checked before any of this, so the window
       * cannot outlive the row it is on: at the refresh token's own lifetime
       * it reads as "replayable for as long as the token is valid at all",
       * which is the sentence the setting should say. The columns this writes
       * to (`rotatedAt`, `rotationReplayExpiresAt`, `rotationReplayResponse`)
       * are already on `oauthRefreshToken` from `0001`, so this needs no
       * migration: they have been sitting unused.
       */
      refreshTokenReuseInterval: REFRESH_TOKEN_TTL_SECONDS,
      /**
       * The granted scopes, handed back with the claims.
       *
       * `/oauth2/userinfo` is where `/api/connectors` turns a bearer token into
       * a person, and OpenID does not put the scope in that answer. Without
       * this, `connectors` would be a word in a consent screen that nothing
       * could check, and a token issued for a narrower purpose later would
       * reach the connector endpoints anyway. The plugin hands the granted
       * scopes to this callback for exactly that reason.
       */
      customUserInfoClaims: ({ scopes }: { scopes: readonly string[] }) => ({
        scope: scopes.join(" "),
      }),
      /**
       * No ID token, and so no signing key to publish or rotate.
       *
       * The app never reads claims out of a token: it asks `/oauth2/userinfo`,
       * which validates the access token against storage on every call. That is
       * the whole reason to prefer it here. A signed JWT is good until it
       * expires whatever this origin later decides, and this is a service whose
       * one job is holding credentials that must be able to die immediately.
       */
      disableJwtPlugin: true,
    }),
    /**
     * The way in. A code to an address, and no password anywhere.
     *
     * This is deliberately the only sign-in method. Identity here is an email
     * address, and every provider is a grant hanging off it, which is what
     * makes "signed in with Google but Google is not authorized" a state that
     * cannot happen. It also means one Google consent screen rather than two.
     *
     * `env` is optional because `schema.test.ts` builds this list without one
     * to ask what tables the plugins want. Nothing there sends.
     */
    emailOTP({
      otpLength: 6,
      expiresIn: CODE_TTL_MINUTES * 60,
      /**
       * Hashed, so the `verification` row is not a live credential.
       *
       * The plain default would make a D1 read, a backup or a leaked snapshot
       * into a working sign-in for anyone who got it inside the window. The
       * cost is that nothing can read a code back, including the tests, which
       * take it from the same place a person does: the send.
       */
      storeOTP: "hashed",
      // Three guesses at a six-digit code, then the code is spent. That is
      // the brute-force answer, and it holds wherever the guess arrives from
      // because it is counted on the stored code rather than on a route.
      //
      // The plugin's own `rateLimit` is deliberately not set: it is applied
      // by Better Auth's router, and nothing here reaches a code through the
      // router. The Worker calls these endpoints directly and their HTTP
      // paths are disabled, so the limit that matters is the one in
      // `codes.ts`, on the form that people actually post.
      allowedAttempts: 3,
      sendVerificationOTP: async ({ email, otp, type }) => {
        // This origin asks for exactly one kind of code. Anything else means a
        // flow was enabled without anyone deciding what its email says.
        if (type !== "sign-in") throw new Error(`unexpected OTP type: ${type}`);
        if (!env) throw new Error("no environment to send from");
        await sendSignInCode(env, email, otp);
      },
    }),
  ] as const;
}

function build(env: Env) {
  const baseURL = env.BETTER_AUTH_URL;
  if (!baseURL) throw new Error("BETTER_AUTH_URL is not set");
  if (!env.BETTER_AUTH_SECRET) throw new Error("BETTER_AUTH_SECRET is not set");

  return betterAuth({
    appName: "Guaca",
    baseURL,
    secret: env.BETTER_AUTH_SECRET,
    basePath: "/api/auth",
    trustedOrigins: [baseURL],
    database: drizzleAdapter(drizzle(env.DB, { schema }), { provider: "sqlite", schema }),

    account: ACCOUNT_OPTIONS,

    session: { expiresIn: SESSION_TTL_DAYS * 24 * 60 * 60 },

    // Nothing here needs a password, and a password is the one credential this
    // service would then have to protect. The way in is `emailOTP`, below.
    emailAndPassword: { enabled: false },

    socialProviders: socialProviders(env),

    disabledPaths: DISABLED_PATHS,

    plugins: [...authPlugins(env)],
  });
}

/**
 * Only providers this deployment holds both halves of a credential for.
 *
 * Configuring a provider with an undefined client id gets you a card that
 * leads to a Google error page, which reads to a visitor as a broken product
 * rather than an unfinished one.
 */
function socialProviders(env: Env) {
  const out: Record<string, { clientId: string; clientSecret: string; accessType?: "offline" }> =
    {};
  for (const provider of PROVIDERS) {
    const [idKey, secretKey] = provider.credentials;
    const clientId = (env as unknown as Record<string, string | undefined>)[idKey];
    const clientSecret = (env as unknown as Record<string, string | undefined>)[secretKey];
    if (!clientId || !clientSecret) continue;
    out[provider.id] = { clientId, clientSecret };
  }
  return out;
}
